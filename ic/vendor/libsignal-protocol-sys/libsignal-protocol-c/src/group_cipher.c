#include "group_cipher.h"

#include <assert.h>
#include <string.h>
#include "protocol.h"
#include "sender_key.h"
#include "sender_key_record.h"
#include "sender_key_state.h"
#include "signal_protocol_internal.h"

struct group_cipher
{
    signal_protocol_store_context *store;
    const signal_protocol_sender_key_name *sender_key_id;
    signal_context *global_context;
    int (*decrypt_callback)(group_cipher *cipher, signal_buffer *plaintext, void *decrypt_context);
    int inside_callback;
    void *user_data;
};

static int group_cipher_get_sender_key(group_cipher *cipher, sender_message_key **sender_key, sender_key_state *state, uint32_t iteration);
static int group_cipher_decrypt_callback(group_cipher *cipher, signal_buffer *plaintext, void *decrypt_context);

int group_cipher_create(group_cipher **cipher,
        signal_protocol_store_context *store, const signal_protocol_sender_key_name *sender_key_id,
        signal_context *global_context)
{
    group_cipher *result_cipher;

    assert(store);
    assert(global_context);

    result_cipher = malloc(sizeof(group_cipher));
    if(!result_cipher) {
        return SG_ERR_NOMEM;
    }
    memset(result_cipher, 0, sizeof(group_cipher));

    result_cipher->store = store;
    result_cipher->sender_key_id = sender_key_id;
    result_cipher->global_context = global_context;

    *cipher = result_cipher;
    return 0;
}

void group_cipher_set_user_data(group_cipher *cipher, void *user_data)
{
    assert(cipher);
    cipher->user_data = user_data;
}

void *group_cipher_get_user_data(group_cipher *cipher)
{
    assert(cipher);
    return cipher->user_data;
}

void group_cipher_set_decryption_callback(group_cipher *cipher,
        int (*callback)(group_cipher *cipher, signal_buffer *plaintext, void *decrypt_context))
{
    assert(cipher);
    cipher->decrypt_callback = callback;
}

int group_cipher_encrypt(group_cipher *cipher,
        const uint8_t *padded_plaintext, size_t padded_plaintext_len,
        ciphertext_message **encrypted_message)
{
    int result = 0;
    sender_key_message *result_message = 0;
    sender_key_record *record = 0;
    sender_key_state *state = 0;
    ec_private_key *signing_key_private = 0;
    sender_message_key *sender_key = 0;
    sender_chain_key *next_chain_key = 0;
    signal_buffer *sender_cipher_key = 0;
    signal_buffer *sender_cipher_iv = 0;
    signal_buffer *ciphertext = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    if(cipher->inside_callback == 1) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = signal_protocol_sender_key_load_key(cipher->store, &record, cipher->sender_key_id);
    if(result < 0) {
        goto complete;
    }

    result = sender_key_record_get_sender_key_state(record, &state);
    if(result < 0) {
        goto complete;
    }

    signing_key_private = sender_key_state_get_signing_key_private(state);
    if(!signing_key_private) {
        result = SG_ERR_INVALID_KEY;
        goto complete;
    }

    result = sender_chain_key_create_message_key(sender_key_state_get_chain_key(state), &sender_key);
    if(result < 0) {
        goto complete;
    }

    sender_cipher_key = sender_message_key_get_cipher_key(sender_key);
    sender_cipher_iv = sender_message_key_get_iv(sender_key);

    result = signal_encrypt(cipher->global_context, &ciphertext, SG_CIPHER_AES_CBC_PKCS5,
            signal_buffer_data(sender_cipher_key), signal_buffer_len(sender_cipher_key),
            signal_buffer_data(sender_cipher_iv), signal_buffer_len(sender_cipher_iv),
            padded_plaintext, padded_plaintext_len);
    if(result < 0) {
        goto complete;
    }

    result = sender_key_message_create(&result_message,
            sender_key_state_get_key_id(state),
            sender_message_key_get_iteration(sender_key),
            signal_buffer_data(ciphertext), signal_buffer_len(ciphertext),
            signing_key_private,
            cipher->global_context);
    if(result < 0) {
        goto complete;
    }

    result = sender_chain_key_create_next(sender_key_state_get_chain_key(state), &next_chain_key);
    if(result < 0) {
        goto complete;
    }

    sender_key_state_set_chain_key(state, next_chain_key);

    result = signal_protocol_sender_key_store_key(cipher->store, cipher->sender_key_id, record);

complete:
    if(result >= 0) {
        *encrypted_message = (ciphertext_message *)result_message;
    }
    else {
        if(result == SG_ERR_INVALID_KEY_ID) {
            result = SG_ERR_NO_SESSION;
        }
        SIGNAL_UNREF(result_message);
    }
    signal_buffer_free(ciphertext);
    SIGNAL_UNREF(next_chain_key);
    SIGNAL_UNREF(sender_key);
    SIGNAL_UNREF(record);
    signal_unlock(cipher->global_context);
    return result;
}

int group_cipher_decrypt(group_cipher *cipher,
        sender_key_message *ciphertext, void *decrypt_context,
        signal_buffer **plaintext)
{
    int result = 0;
    signal_buffer *result_buf = 0;
    sender_key_record *record = 0;
    sender_key_state *state = 0;
    sender_message_key *sender_key = 0;
    signal_buffer *sender_cipher_key = 0;
    signal_buffer *sender_cipher_iv = 0;
    signal_buffer *ciphertext_body = 0;

    assert(cipher);
    signal_lock(cipher->global_context);

    if(cipher->inside_callback == 1) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = signal_protocol_sender_key_load_key(cipher->store, &record, cipher->sender_key_id);
    if(result < 0) {
        goto complete;
    }

    if(sender_key_record_is_empty(record)) {
        result = SG_ERR_NO_SESSION;
        signal_log(cipher->global_context, SG_LOG_WARNING, "No sender key for: %s::%s::%d",
                cipher->sender_key_id->group_id,
                cipher->sender_key_id->sender.name,
                cipher->sender_key_id->sender.device_id);
        goto complete;
    }

    result = sender_key_record_get_sender_key_state_by_id(record, &state, sender_key_message_get_key_id(ciphertext));
    if(result < 0) {
        goto complete;
    }

    result = sender_key_message_verify_signature(ciphertext, sender_key_state_get_signing_key_public(state));
    if(result < 0) {
        goto complete;
    }

    result = group_cipher_get_sender_key(cipher, &sender_key, state, sender_key_message_get_iteration(ciphertext));
    if(result < 0) {
        goto complete;
    }

    sender_cipher_key = sender_message_key_get_cipher_key(sender_key);
    sender_cipher_iv = sender_message_key_get_iv(sender_key);
    ciphertext_body = sender_key_message_get_ciphertext(ciphertext);

    result = signal_decrypt(cipher->global_context, &result_buf, SG_CIPHER_AES_CBC_PKCS5,
            signal_buffer_data(sender_cipher_key), signal_buffer_len(sender_cipher_key),
            signal_buffer_data(sender_cipher_iv), signal_buffer_len(sender_cipher_iv),
            signal_buffer_data(ciphertext_body), signal_buffer_len(ciphertext_body));
    if(result < 0) {
        goto complete;
    }

    result = group_cipher_decrypt_callback(cipher, result_buf, decrypt_context);
    if(result < 0) {
        goto complete;
    }

    result = signal_protocol_sender_key_store_key(cipher->store, cipher->sender_key_id, record);

complete:
    SIGNAL_UNREF(sender_key);
    SIGNAL_UNREF(record);
    if(result >= 0) {
        *plaintext = result_buf;
    }
    else {
        if(result == SG_ERR_INVALID_KEY || result == SG_ERR_INVALID_KEY_ID) {
            result = SG_ERR_INVALID_MESSAGE;
        }
        signal_buffer_free(result_buf);
    }
    signal_unlock(cipher->global_context);
    return result;
}

int group_cipher_get_sender_key(group_cipher *cipher, sender_message_key **sender_key, sender_key_state *state, uint32_t iteration)
{
    int result = 0;
    sender_message_key *result_key = 0;
    sender_chain_key *chain_key = 0;
    sender_chain_key *next_chain_key = 0;
    sender_message_key *message_key = 0;

    chain_key = sender_key_state_get_chain_key(state);
    SIGNAL_REF(chain_key);

    if(sender_chain_key_get_iteration(chain_key) > iteration) {
        if(sender_key_state_has_sender_message_key(state, iteration)) {
            result_key = sender_key_state_remove_sender_message_key(state, iteration);
            if(!result_key) {
                result = SG_ERR_UNKNOWN;
            }
            goto complete;
        }
        else {
            result = SG_ERR_DUPLICATE_MESSAGE;
            signal_log(cipher->global_context, SG_LOG_WARNING,
                    "Received message with old counter: %d, %d",
                    sender_chain_key_get_iteration(chain_key), iteration);
            goto complete;
        }
    }

    if(iteration - sender_chain_key_get_iteration(chain_key) > 2000) {
        result = SG_ERR_INVALID_MESSAGE;
        signal_log(cipher->global_context, SG_LOG_WARNING, "Over 2000 messages into the future!");
        goto complete;
    }

    while(sender_chain_key_get_iteration(chain_key) < iteration) {
        result = sender_chain_key_create_message_key(chain_key, &message_key);
        if(result < 0) {
            goto complete;
        }

        result = sender_key_state_add_sender_message_key(state, message_key);
        if(result < 0) {
            goto complete;
        }
        SIGNAL_UNREF(message_key);

        result = sender_chain_key_create_next(chain_key, &next_chain_key);
        if(result < 0) {
            goto complete;
        }

        SIGNAL_UNREF(chain_key);
        chain_key = next_chain_key;
        next_chain_key = 0;
    }

    result = sender_chain_key_create_next(chain_key, &next_chain_key);
    if(result < 0) {
        goto complete;
    }

    sender_key_state_set_chain_key(state, next_chain_key);
    result = sender_chain_key_create_message_key(chain_key, &result_key);

complete:
    SIGNAL_UNREF(message_key);
    SIGNAL_UNREF(chain_key);
    SIGNAL_UNREF(next_chain_key);
    if(result >= 0) {
        *sender_key = result_key;
    }
    return result;
}

static int group_cipher_decrypt_callback(group_cipher *cipher, signal_buffer *plaintext, void *decrypt_context)
{
    int result = 0;
    if(cipher->decrypt_callback) {
        cipher->inside_callback = 1;
        result = cipher->decrypt_callback(cipher, plaintext, decrypt_context);
        cipher->inside_callback = 0;
    }
    return result;
}

void group_cipher_free(group_cipher *cipher)
{
    if(cipher) {
        free(cipher);
    }
}
