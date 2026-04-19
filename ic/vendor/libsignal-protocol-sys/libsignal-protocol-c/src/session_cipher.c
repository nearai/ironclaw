#include "session_cipher.h"

#include <assert.h>
#include <string.h>
#include "session_builder.h"
#include "session_builder_internal.h"
#include "session_record.h"
#include "session_state.h"
#include "ratchet.h"
#include "protocol.h"
#include "signal_protocol_internal.h"

struct session_cipher
{
    signal_protocol_store_context *store;
    const signal_protocol_address *remote_address;
    session_builder *builder;
    signal_context *global_context;
    int (*decrypt_callback)(session_cipher *cipher, signal_buffer *plaintext, void *decrypt_context);
    int inside_callback;
    void *user_data;
};

static int session_cipher_decrypt_from_record_and_signal_message(session_cipher *cipher,
        session_record *record, signal_message *ciphertext, signal_buffer **plaintext);
static int session_cipher_decrypt_from_state_and_signal_message(session_cipher *cipher,
        session_state *state, signal_message *ciphertext, signal_buffer **plaintext);

static int session_cipher_get_or_create_chain_key(session_cipher *cipher,
        ratchet_chain_key **chain_key,
        session_state *state, ec_public_key *their_ephemeral);
static int session_cipher_get_or_create_message_keys(ratchet_message_keys *message_keys,
        session_state *state, ec_public_key *their_ephemeral,
        ratchet_chain_key *chain_key, uint32_t counter,
        signal_context *global_context);

static int session_cipher_get_ciphertext(session_cipher *cipher,
        signal_buffer **ciphertext,
        uint32_t version, ratchet_message_keys *message_keys,
        const uint8_t *plaintext, size_t plaintext_len);
static int session_cipher_get_plaintext(session_cipher *cipher,
        signal_buffer **plaintext,
        uint32_t version, ratchet_message_keys *message_keys,
        const uint8_t *ciphertext, size_t ciphertext_len);

static int session_cipher_decrypt_callback(session_cipher *cipher, signal_buffer *plaintext, void *decrypt_context);

int session_cipher_create(session_cipher **cipher,
        signal_protocol_store_context *store, const signal_protocol_address *remote_address,
        signal_context *global_context)
{
    int result = 0;
    session_builder *builder = 0;
    session_cipher *result_cipher;

    assert(store);
    assert(global_context);

    result = session_builder_create(&builder, store, remote_address, global_context);
    if(result < 0) {
        return result;
    }

    result_cipher = malloc(sizeof(session_cipher));
    if(!result_cipher) {
        return SG_ERR_NOMEM;
    }
    memset(result_cipher, 0, sizeof(session_cipher));

    result_cipher->store = store;
    result_cipher->remote_address = remote_address;
    result_cipher->builder = builder;
    result_cipher->global_context = global_context;

    *cipher = result_cipher;
    return 0;
}

void session_cipher_set_user_data(session_cipher *cipher, void *user_data)
{
    assert(cipher);
    cipher->user_data = user_data;
}

void *session_cipher_get_user_data(session_cipher *cipher)
{
    assert(cipher);
    return cipher->user_data;
}

void session_cipher_set_decryption_callback(session_cipher *cipher,
        int (*callback)(session_cipher *cipher, signal_buffer *plaintext, void *decrypt_context))
{
    assert(cipher);
    cipher->decrypt_callback = callback;
}

int session_cipher_encrypt(session_cipher *cipher,
        const uint8_t *padded_message, size_t padded_message_len,
        ciphertext_message **encrypted_message)
{
    int result = 0;
    session_record *record = 0;
    session_state *state = 0;
    ratchet_chain_key *chain_key = 0;
    ratchet_chain_key *next_chain_key = 0;
    ratchet_message_keys message_keys;
    ec_public_key *sender_ephemeral = 0;
    uint32_t previous_counter = 0;
    uint32_t session_version = 0;
    signal_buffer *ciphertext = 0;
    uint32_t chain_key_index = 0;
    ec_public_key *local_identity_key = 0;
    ec_public_key *remote_identity_key = 0;
    signal_message *message = 0;
    pre_key_signal_message *pre_key_message = 0;
    uint8_t *ciphertext_data = 0;
    size_t ciphertext_len = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    if(cipher->inside_callback == 1) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = signal_protocol_session_load_session(cipher->store, &record, cipher->remote_address);
    if(result < 0) {
        goto complete;
    }

    state = session_record_get_state(record);
    if(!state) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    chain_key = session_state_get_sender_chain_key(state);
    if(!chain_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = ratchet_chain_key_get_message_keys(chain_key, &message_keys);
    if(result < 0) {
        goto complete;
    }

    sender_ephemeral = session_state_get_sender_ratchet_key(state);
    if(!sender_ephemeral) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    previous_counter = session_state_get_previous_counter(state);
    session_version = session_state_get_session_version(state);

    result = session_cipher_get_ciphertext(cipher,
            &ciphertext,
            session_version, &message_keys,
            padded_message, padded_message_len);
    if(result < 0) {
        goto complete;
    }
    ciphertext_data = signal_buffer_data(ciphertext);
    ciphertext_len = signal_buffer_len(ciphertext);

    chain_key_index = ratchet_chain_key_get_index(chain_key);

    local_identity_key = session_state_get_local_identity_key(state);
    if(!local_identity_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    remote_identity_key = session_state_get_remote_identity_key(state);
    if(!remote_identity_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = signal_message_create(&message,
            session_version,
            message_keys.mac_key, sizeof(message_keys.mac_key),
            sender_ephemeral,
            chain_key_index, previous_counter,
            ciphertext_data, ciphertext_len,
            local_identity_key, remote_identity_key,
            cipher->global_context);
    if(result < 0) {
        goto complete;
    }

    if(session_state_has_unacknowledged_pre_key_message(state) == 1) {
        uint32_t local_registration_id = session_state_get_local_registration_id(state);
        int has_pre_key_id = 0;
        uint32_t pre_key_id = 0;
        uint32_t signed_pre_key_id;
        ec_public_key *base_key;
        
        if(session_state_unacknowledged_pre_key_message_has_pre_key_id(state)) {
            has_pre_key_id = 1;
            pre_key_id = session_state_unacknowledged_pre_key_message_get_pre_key_id(state);
        }
        signed_pre_key_id = session_state_unacknowledged_pre_key_message_get_signed_pre_key_id(state);
        base_key = session_state_unacknowledged_pre_key_message_get_base_key(state);

        if(!base_key) {
            result = SG_ERR_UNKNOWN;
            goto complete;
        }

        result = pre_key_signal_message_create(&pre_key_message,
                session_version, local_registration_id, (has_pre_key_id ? &pre_key_id : 0),
                signed_pre_key_id, base_key, local_identity_key,
                message,
                cipher->global_context);
        if(result < 0) {
            goto complete;
        }
        SIGNAL_UNREF(message);
        message = 0;
    }

    result = ratchet_chain_key_create_next(chain_key, &next_chain_key);
    if(result < 0) {
        goto complete;
    }

    result = session_state_set_sender_chain_key(state, next_chain_key);
    if(result < 0) {
        goto complete;
    }

    result = signal_protocol_session_store_session(cipher->store, cipher->remote_address, record);

complete:
    if(result >= 0) {
        if(pre_key_message) {
            *encrypted_message = (ciphertext_message *)pre_key_message;
        }
        else {
            *encrypted_message = (ciphertext_message *)message;
        }
    }
    else {
        SIGNAL_UNREF(pre_key_message);
        SIGNAL_UNREF(message);
    }
    signal_buffer_free(ciphertext);
    SIGNAL_UNREF(next_chain_key);
    SIGNAL_UNREF(record);
    signal_explicit_bzero(&message_keys, sizeof(ratchet_message_keys));
    signal_unlock(cipher->global_context);
    return result;
}

int session_cipher_decrypt_pre_key_signal_message(session_cipher *cipher,
        pre_key_signal_message *ciphertext, void *decrypt_context,
        signal_buffer **plaintext)
{
    int result = 0;
    signal_buffer *result_buf = 0;
    session_record *record = 0;
    int has_unsigned_pre_key_id = 0;
    uint32_t unsigned_pre_key_id = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    if(cipher->inside_callback == 1) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = signal_protocol_session_load_session(cipher->store, &record, cipher->remote_address);
    if(result < 0) {
        goto complete;
    }

    result = session_builder_process_pre_key_signal_message(cipher->builder, record, ciphertext, &unsigned_pre_key_id);
    if(result < 0) {
        goto complete;
    }
    has_unsigned_pre_key_id = result;

    result = session_cipher_decrypt_from_record_and_signal_message(cipher, record,
            pre_key_signal_message_get_signal_message(ciphertext),
            &result_buf);
    if(result < 0) {
        goto complete;
    }

    result = session_cipher_decrypt_callback(cipher, result_buf, decrypt_context);
    if(result < 0) {
        goto complete;
    }

    result = signal_protocol_session_store_session(cipher->store, cipher->remote_address, record);
    if(result < 0) {
        goto complete;
    }

    if(has_unsigned_pre_key_id) {
        result = signal_protocol_pre_key_remove_key(cipher->store, unsigned_pre_key_id);
        if(result < 0) {
            goto complete;
        }
    }

complete:
    SIGNAL_UNREF(record);
    if(result >= 0) {
        *plaintext = result_buf;
    }
    else {
        signal_buffer_free(result_buf);
    }
    signal_unlock(cipher->global_context);
    return result;
}

int session_cipher_decrypt_signal_message(session_cipher *cipher,
        signal_message *ciphertext, void *decrypt_context,
        signal_buffer **plaintext)
{
    int result = 0;
    signal_buffer *result_buf = 0;
    session_record *record = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    if(cipher->inside_callback == 1) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = signal_protocol_session_contains_session(cipher->store, cipher->remote_address);
    if(result == 0) {
        signal_log(cipher->global_context, SG_LOG_WARNING, "No session for: %s:%d", cipher->remote_address->name, cipher->remote_address->device_id);
        result = SG_ERR_NO_SESSION;
        goto complete;
    }
    else if(result < 0) {
        goto complete;
    }

    result = signal_protocol_session_load_session(cipher->store, &record,
            cipher->remote_address);
    if(result < 0) {
        goto complete;
    }

    result = session_cipher_decrypt_from_record_and_signal_message(
            cipher, record, ciphertext, &result_buf);
    if(result < 0) {
        goto complete;
    }

    result = session_cipher_decrypt_callback(cipher, result_buf, decrypt_context);
    if(result < 0) {
        goto complete;
    }

    result = signal_protocol_session_store_session(cipher->store,
            cipher->remote_address, record);

complete:
    SIGNAL_UNREF(record);
    if(result >= 0) {
        *plaintext = result_buf;
    }
    else {
        signal_buffer_free(result_buf);
    }
    signal_unlock(cipher->global_context);
    return result;
}

static int session_cipher_decrypt_from_record_and_signal_message(session_cipher *cipher,
        session_record *record, signal_message *ciphertext, signal_buffer **plaintext)
{
    int result = 0;
    signal_buffer *result_buf = 0;
    session_state *state = 0;
    session_state *state_copy = 0;
    session_record_state_node *previous_states_node = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    state = session_record_get_state(record);
    if(state) {
        result = session_state_copy(&state_copy, state, cipher->global_context);
        if(result < 0) {
            goto complete;
        }

        //TODO Collect and log invalid message errors if totally unsuccessful

        result = session_cipher_decrypt_from_state_and_signal_message(cipher, state_copy, ciphertext, &result_buf);
        if(result < 0 && result != SG_ERR_INVALID_MESSAGE) {
            goto complete;
        }

        if(result >= SG_SUCCESS) {
            session_record_set_state(record, state_copy);
            goto complete;
        }
        SIGNAL_UNREF(state_copy);
    }

    previous_states_node = session_record_get_previous_states_head(record);
    while(previous_states_node) {
        state = session_record_get_previous_states_element(previous_states_node);

        result = session_state_copy(&state_copy, state, cipher->global_context);
        if(result < 0) {
            goto complete;
        }

        result = session_cipher_decrypt_from_state_and_signal_message(cipher, state_copy, ciphertext, &result_buf);
        if(result < 0 && result != SG_ERR_INVALID_MESSAGE) {
            goto complete;
        }

        if(result >= SG_SUCCESS) {
            session_record_get_previous_states_remove(record, previous_states_node);
            result = session_record_promote_state(record, state_copy);
            goto complete;
        }

        SIGNAL_UNREF(state_copy);
        previous_states_node = session_record_get_previous_states_next(previous_states_node);
    }

    signal_log(cipher->global_context, SG_LOG_WARNING, "No valid sessions");
    result = SG_ERR_INVALID_MESSAGE;

complete:
    SIGNAL_UNREF(state_copy);
    if(result >= 0) {
        *plaintext = result_buf;
    }
    else {
        signal_buffer_free(result_buf);
    }
    signal_unlock(cipher->global_context);
    return result;
}

static int session_cipher_decrypt_from_state_and_signal_message(session_cipher *cipher,
        session_state *state, signal_message *ciphertext, signal_buffer **plaintext)
{
    int result = 0;
    signal_buffer *result_buf = 0;
    ec_public_key *their_ephemeral = 0;
    uint32_t counter = 0;
    ratchet_chain_key *chain_key = 0;
    ratchet_message_keys message_keys;
    uint8_t message_version = 0;
    uint32_t session_version = 0;
    ec_public_key *remote_identity_key = 0;
    ec_public_key *local_identity_key = 0;
    signal_buffer *ciphertext_body = 0;

    if(!session_state_has_sender_chain(state)) {
        signal_log(cipher->global_context, SG_LOG_WARNING, "Uninitialized session!");
        result = SG_ERR_INVALID_MESSAGE;
        goto complete;
    }

    message_version = signal_message_get_message_version(ciphertext);
    session_version = session_state_get_session_version(state);

    if(message_version != session_version) {
        signal_log(cipher->global_context, SG_LOG_WARNING, "Message version %d, but session version %d", message_version, session_version);
        result = SG_ERR_INVALID_MESSAGE;
        goto complete;
    }

    their_ephemeral = signal_message_get_sender_ratchet_key(ciphertext);
    if(!their_ephemeral) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    counter = signal_message_get_counter(ciphertext);

    result = session_cipher_get_or_create_chain_key(cipher, &chain_key, state, their_ephemeral);
    if(result < 0) {
        goto complete;
    }

    result = session_cipher_get_or_create_message_keys(&message_keys, state,
            their_ephemeral, chain_key, counter, cipher->global_context);
    if(result < 0) {
        goto complete;
    }

    remote_identity_key = session_state_get_remote_identity_key(state);
    if(!remote_identity_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    local_identity_key = session_state_get_local_identity_key(state);
    if(!local_identity_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = signal_message_verify_mac(ciphertext,
            remote_identity_key, local_identity_key,
            message_keys.mac_key, sizeof(message_keys.mac_key),
            cipher->global_context);
    if(result != 1) {
        if(result == 0) {
            signal_log(cipher->global_context, SG_LOG_WARNING, "Message mac not verified");
            result = SG_ERR_INVALID_MESSAGE;
        }
        else if(result < 0) {
            signal_log(cipher->global_context, SG_LOG_WARNING, "Error attempting to verify message mac");
        }
        goto complete;
    }

    ciphertext_body = signal_message_get_body(ciphertext);
    if(!ciphertext_body) {
        signal_log(cipher->global_context, SG_LOG_WARNING, "Message body does not exist");
        result = SG_ERR_INVALID_MESSAGE;
        goto complete;
    }

    result = session_cipher_get_plaintext(cipher, &result_buf, message_version, &message_keys,
            signal_buffer_data(ciphertext_body), signal_buffer_len(ciphertext_body));
    if(result < 0) {
        goto complete;
    }

    session_state_clear_unacknowledged_pre_key_message(state);

complete:
    SIGNAL_UNREF(chain_key);
    if(result >= 0) {
        *plaintext = result_buf;
    }
    else {
        signal_buffer_free(result_buf);
    }
    signal_explicit_bzero(&message_keys, sizeof(ratchet_message_keys));
    return result;
}

static int session_cipher_get_or_create_chain_key(session_cipher *cipher,
        ratchet_chain_key **chain_key,
        session_state *state, ec_public_key *their_ephemeral)
{
    int result = 0;
    ratchet_chain_key *result_key = 0;
    ratchet_root_key *receiver_root_key = 0;
    ratchet_chain_key *receiver_chain_key = 0;
    ratchet_root_key *sender_root_key = 0;
    ratchet_chain_key *sender_chain_key = 0;
    ec_key_pair *our_new_ephemeral = 0;
    ratchet_root_key *root_key = 0;
    ec_key_pair *our_ephemeral = 0;
    ratchet_chain_key *previous_sender_chain_key = 0;
    uint32_t index = 0;

    result_key = session_state_get_receiver_chain_key(state, their_ephemeral);
    if(result_key) {
        SIGNAL_REF(result_key);
        goto complete;
    }

    root_key = session_state_get_root_key(state);
    if(!root_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    our_ephemeral = session_state_get_sender_ratchet_key_pair(state);
    if(!our_ephemeral) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = ratchet_root_key_create_chain(root_key,
            &receiver_root_key, &receiver_chain_key,
            their_ephemeral, ec_key_pair_get_private(our_ephemeral));
    if(result < 0) {
        goto complete;
    }

    result = curve_generate_key_pair(cipher->global_context, &our_new_ephemeral);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_root_key_create_chain(receiver_root_key,
            &sender_root_key, &sender_chain_key,
            their_ephemeral, ec_key_pair_get_private(our_new_ephemeral));
    if(result < 0) {
        goto complete;
    }

    session_state_set_root_key(state, sender_root_key);

    result = session_state_add_receiver_chain(state, their_ephemeral, receiver_chain_key);
    if(result < 0) {
        goto complete;
    }

    previous_sender_chain_key = session_state_get_sender_chain_key(state);
    if(!previous_sender_chain_key) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    index = ratchet_chain_key_get_index(previous_sender_chain_key);
    if(index > 0) { --index; }

    session_state_set_previous_counter(state, index);
    session_state_set_sender_chain(state, our_new_ephemeral, sender_chain_key);

    result_key = receiver_chain_key;
    SIGNAL_REF(result_key);

complete:
    SIGNAL_UNREF(receiver_root_key);
    SIGNAL_UNREF(receiver_chain_key);
    SIGNAL_UNREF(sender_root_key);
    SIGNAL_UNREF(sender_chain_key);
    SIGNAL_UNREF(our_new_ephemeral);
    if(result >= 0) {
        *chain_key = result_key;
    }
    else {
        SIGNAL_UNREF(result_key);
    }
    return result;
}

static int session_cipher_get_or_create_message_keys(ratchet_message_keys *message_keys,
        session_state *state, ec_public_key *their_ephemeral,
        ratchet_chain_key *chain_key, uint32_t counter, signal_context *global_context)
{
    int result = 0;
    ratchet_chain_key *cur_chain_key = 0;
    ratchet_chain_key *next_chain_key = 0;
    ratchet_message_keys message_keys_result;

    if(ratchet_chain_key_get_index(chain_key) > counter) {
        result = session_state_remove_message_keys(state, &message_keys_result, their_ephemeral, counter);
        if(result == 1) {
            result = 0;
            goto complete;
        }

        signal_log(global_context, SG_LOG_WARNING, "Received message with old counter: %d, %d",
                ratchet_chain_key_get_index(chain_key), counter);
        result = SG_ERR_DUPLICATE_MESSAGE;
        goto complete;
    }

    if(counter - ratchet_chain_key_get_index(chain_key) > 2000) {
        signal_log(global_context, SG_LOG_WARNING, "Over 2000 messages into the future!");
        result = SG_ERR_INVALID_MESSAGE;
        goto complete;
    }

    cur_chain_key = chain_key;
    SIGNAL_REF(cur_chain_key);

    while(ratchet_chain_key_get_index(cur_chain_key) < counter) {
        result = ratchet_chain_key_get_message_keys(cur_chain_key, &message_keys_result);
        if(result < 0) {
            goto complete;
        }

        result = session_state_set_message_keys(state, their_ephemeral, &message_keys_result);
        if(result < 0) {
            goto complete;
        }

        result = ratchet_chain_key_create_next(cur_chain_key, &next_chain_key);
        if(result < 0) {
            goto complete;
        }
        SIGNAL_UNREF(cur_chain_key);
        cur_chain_key = next_chain_key;
        next_chain_key = 0;
    }

    result = ratchet_chain_key_create_next(cur_chain_key, &next_chain_key);
    if(result < 0) {
        goto complete;
    }

    result = session_state_set_receiver_chain_key(state, their_ephemeral, next_chain_key);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_chain_key_get_message_keys(cur_chain_key, &message_keys_result);
    if(result < 0) {
        goto complete;
    }

complete:
    if(result >= 0) {
        memcpy(message_keys, &message_keys_result, sizeof(ratchet_message_keys));
    }
    SIGNAL_UNREF(cur_chain_key);
    SIGNAL_UNREF(next_chain_key);
    signal_explicit_bzero(&message_keys_result, sizeof(ratchet_message_keys));
    return result;
}

int session_cipher_get_remote_registration_id(session_cipher *cipher, uint32_t *remote_id)
{
    int result = 0;
    uint32_t id_result = 0;
    session_record *record = 0;
    session_state *state = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    result = signal_protocol_session_load_session(cipher->store, &record, cipher->remote_address);
    if(result < 0) {
        goto complete;
    }

    state = session_record_get_state(record);
    if(!state) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    id_result = session_state_get_remote_registration_id(state);

complete:
    if(result >= 0) {
        *remote_id = id_result;
    }
    signal_unlock(cipher->global_context);
    return result;
}

int session_cipher_get_session_version(session_cipher *cipher, uint32_t *version)
{
    int result = 0;
    uint32_t version_result = 0;
    session_record *record = 0;
    session_state *state = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    result = signal_protocol_session_contains_session(cipher->store, cipher->remote_address);
    if(result != 1) {
        if(result == 0) {
            signal_log(cipher->global_context, SG_LOG_WARNING, "No session for: %s:%d", cipher->remote_address->name, cipher->remote_address->device_id);
            result = SG_ERR_NO_SESSION;
        }
        goto complete;
    }

    result = signal_protocol_session_load_session(cipher->store, &record, cipher->remote_address);
    if(result < 0) {
        goto complete;
    }

    state = session_record_get_state(record);
    if(!state) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    version_result = session_state_get_session_version(state);

complete:
    if(result >= 0) {
        *version = version_result;
    }
    signal_unlock(cipher->global_context);
    return result;
}

static int session_cipher_get_ciphertext(session_cipher *cipher,
        signal_buffer **ciphertext,
        uint32_t version, ratchet_message_keys *message_keys,
        const uint8_t *plaintext, size_t plaintext_len)
{
    int result = 0;
    signal_buffer *output = 0;

    if(version >= 3) {
        result = signal_encrypt(cipher->global_context,
                &output, SG_CIPHER_AES_CBC_PKCS5,
                message_keys->cipher_key, sizeof(message_keys->cipher_key),
                message_keys->iv, sizeof(message_keys->iv),
                plaintext, plaintext_len);
    }
    else {
        uint8_t iv[16];
        memset(iv, 0, sizeof(iv));
        iv[3] = (uint8_t)(message_keys->counter);
        iv[2] = (uint8_t)(message_keys->counter >> 8);
        iv[1] = (uint8_t)(message_keys->counter >> 16);
        iv[0] = (uint8_t)(message_keys->counter >> 24);

        result = signal_encrypt(cipher->global_context,
                &output, SG_CIPHER_AES_CTR_NOPADDING,
                message_keys->cipher_key, sizeof(message_keys->cipher_key),
                iv, sizeof(iv),
                plaintext, plaintext_len);
    }

    if(result >= 0) {
        *ciphertext = output;
    }

    return result;
}

static int session_cipher_get_plaintext(session_cipher *cipher,
        signal_buffer **plaintext,
        uint32_t version, ratchet_message_keys *message_keys,
        const uint8_t *ciphertext, size_t ciphertext_len)
{
    int result = 0;
    signal_buffer *output = 0;

    if(version >= 3) {
        result = signal_decrypt(cipher->global_context,
                &output, SG_CIPHER_AES_CBC_PKCS5,
                message_keys->cipher_key, sizeof(message_keys->cipher_key),
                message_keys->iv, sizeof(message_keys->iv),
                ciphertext, ciphertext_len);
    }
    else {
        uint8_t iv[16];
        memset(iv, 0, sizeof(iv));
        iv[3] = (uint8_t)(message_keys->counter);
        iv[2] = (uint8_t)(message_keys->counter >> 8);
        iv[1] = (uint8_t)(message_keys->counter >> 16);
        iv[0] = (uint8_t)(message_keys->counter >> 24);

        result = signal_decrypt(cipher->global_context,
                &output, SG_CIPHER_AES_CTR_NOPADDING,
                message_keys->cipher_key, sizeof(message_keys->cipher_key),
                iv, sizeof(iv),
                ciphertext, ciphertext_len);
    }

    if(result >= 0) {
        *plaintext = output;
    }

    return result;
}

static int session_cipher_decrypt_callback(session_cipher *cipher, signal_buffer *plaintext, void *decrypt_context)
{
    int result = 0;
    if(cipher->decrypt_callback) {
        cipher->inside_callback = 1;
        result = cipher->decrypt_callback(cipher, plaintext, decrypt_context);
        cipher->inside_callback = 0;
    }
    return result;
}

void session_cipher_free(session_cipher *cipher)
{
    if(cipher) {
        session_builder_free(cipher->builder);
        free(cipher);
    }
}
