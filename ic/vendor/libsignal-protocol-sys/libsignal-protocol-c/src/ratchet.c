#include "ratchet.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <protobuf-c/protobuf-c.h>

#include "hkdf.h"
#include "curve.h"
#include "session_state.h"
#include "protocol.h"
#include "vpool.h"
#include "LocalStorageProtocol.pb-c.h"
#include "signal_protocol_internal.h"

#define HASH_OUTPUT_SIZE 32
#define DERIVED_MESSAGE_SECRETS_SIZE 80
#define DERIVED_ROOT_SECRETS_SIZE 64

struct ratchet_chain_key {
    signal_type_base base;
    signal_context *global_context;
    hkdf_context *kdf;
    uint8_t *key;
    size_t key_len;
    uint32_t index;
};

struct ratchet_root_key {
    signal_type_base base;
    signal_context *global_context;
    hkdf_context *kdf;
    uint8_t *key;
    size_t key_len;
};

struct ratchet_identity_key_pair {
    signal_type_base base;
    ec_public_key *public_key;
    ec_private_key *private_key;
};

int ratchet_chain_key_create(ratchet_chain_key **chain_key, hkdf_context *kdf, const uint8_t *key, size_t key_len, uint32_t index, signal_context *global_context)
{
    ratchet_chain_key *result = 0;

    if(!kdf || !key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(ratchet_chain_key));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    SIGNAL_INIT(result, ratchet_chain_key_destroy);
    result->global_context = global_context;
    result->kdf = kdf;

    result->key = malloc(key_len);
    if(!result->key) {
        free(result);
        return SG_ERR_NOMEM;
    }
    memcpy(result->key, key, key_len);
    result->key_len = key_len;

    result->index = index;
    SIGNAL_REF(result->kdf);
    *chain_key = result;

    return 0;
}

int ratchet_chain_key_get_key(const ratchet_chain_key *chain_key, signal_buffer **buffer)
{
    signal_buffer *buf = 0;
    uint8_t *data = 0;
    
    buf = signal_buffer_alloc(chain_key->key_len);
    if(!buf) {
        return SG_ERR_NOMEM;
    }

    data = signal_buffer_data(buf);
    memcpy(data, chain_key->key, chain_key->key_len);

    *buffer = buf;

    return 0;
}

int ratchet_chain_key_get_key_protobuf(const ratchet_chain_key *chain_key, ProtobufCBinaryData *buffer)
{
    uint8_t *data = 0;

    assert(chain_key);
    assert(buffer);

    data = malloc(chain_key->key_len);
    if(!data) {
        return SG_ERR_NOMEM;
    }

    memcpy(data, chain_key->key, chain_key->key_len);

    buffer->data = data;
    buffer->len = chain_key->key_len;
    return 0;
}

uint32_t ratchet_chain_key_get_index(const ratchet_chain_key *chain_key)
{
    return chain_key->index;
}

ssize_t ratchet_chain_key_get_base_material(const ratchet_chain_key *chain_key, uint8_t **material, const uint8_t *seed, size_t seed_len)
{
    int result = 0;
    signal_buffer *output_buffer = 0;
    uint8_t *output = 0;
    size_t output_len = 0;

    void *hmac_context = 0;
    result = signal_hmac_sha256_init(chain_key->global_context, &hmac_context, chain_key->key, chain_key->key_len);
    if(result < 0) {
        goto complete;
    }

    result = signal_hmac_sha256_update(chain_key->global_context, hmac_context, seed, seed_len);
    if(result < 0) {
        goto complete;
    }

    result = signal_hmac_sha256_final(chain_key->global_context, hmac_context, &output_buffer);
    if(result < 0) {
        goto complete;
    }

    output_len = signal_buffer_len(output_buffer);
    output = malloc(output_len);
    if(!output) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    memcpy(output, signal_buffer_data(output_buffer), output_len);

complete:
    signal_hmac_sha256_cleanup(chain_key->global_context, hmac_context);
    signal_buffer_free(output_buffer);

    if(result >= 0) {
        *material = output;
        return (ssize_t)output_len;
    }
    else {
        return result;
    }
}

int ratchet_chain_key_get_message_keys(ratchet_chain_key *chain_key, ratchet_message_keys *message_keys)
{
    static const uint8_t message_key_seed = 0x01;
    static const char key_material_seed[] = "WhisperMessageKeys";
    uint8_t salt[HASH_OUTPUT_SIZE];
    int result = 0;
    ssize_t result_size = 0;
    uint8_t *input_key_material = 0;
    size_t input_key_material_len = 0;
    uint8_t *key_material_data = 0;
    size_t key_material_data_len = 0;

    memset(message_keys, 0, sizeof(ratchet_message_keys));

    result_size = ratchet_chain_key_get_base_material(chain_key, &input_key_material, &message_key_seed, sizeof(message_key_seed));
    if(result_size < 0) {
        result = (int)result_size;
        signal_log(chain_key->global_context, SG_LOG_WARNING, "ratchet_chain_key_get_base_material failed");
        goto complete;
    }
    input_key_material_len = (size_t)result_size;

    memset(salt, 0, sizeof(salt));

    result_size = hkdf_derive_secrets(chain_key->kdf,
            &key_material_data,
            input_key_material, input_key_material_len,
            salt, sizeof(salt),
            (uint8_t *)key_material_seed, sizeof(key_material_seed) - 1,
            DERIVED_MESSAGE_SECRETS_SIZE);
    if(result_size < 0) {
        result = (int)result_size;
        signal_log(chain_key->global_context, SG_LOG_WARNING, "hkdf_derive_secrets failed");
        goto complete;
    }
    key_material_data_len = (size_t)result_size;

    if(key_material_data_len != RATCHET_CIPHER_KEY_LENGTH + RATCHET_MAC_KEY_LENGTH + RATCHET_IV_LENGTH) {
        signal_log(chain_key->global_context, SG_LOG_WARNING,
                "key_material_data length mismatch: %d != %d",
                key_material_data_len, (RATCHET_CIPHER_KEY_LENGTH + RATCHET_MAC_KEY_LENGTH + RATCHET_IV_LENGTH));
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    memcpy(message_keys->cipher_key, key_material_data, RATCHET_CIPHER_KEY_LENGTH);
    memcpy(message_keys->mac_key, key_material_data + RATCHET_CIPHER_KEY_LENGTH, RATCHET_MAC_KEY_LENGTH);
    memcpy(message_keys->iv, key_material_data + RATCHET_CIPHER_KEY_LENGTH + RATCHET_MAC_KEY_LENGTH, RATCHET_IV_LENGTH);
    message_keys->counter = chain_key->index;

complete:
    if(input_key_material) {
        free(input_key_material);
    }
    if(key_material_data) {
        free(key_material_data);
    }
    if(result < 0) {
        return result;
    }
    else {
        return 0;
    }
}

int ratchet_chain_key_create_next(const ratchet_chain_key *chain_key, ratchet_chain_key **next_chain_key)
{
    static const uint8_t chain_key_seed = 0x02;
    int result = 0;
    ssize_t result_size = 0;
    uint8_t *next_key = 0;
    size_t next_key_len = 0;

    result_size = ratchet_chain_key_get_base_material(chain_key, &next_key, &chain_key_seed, sizeof(chain_key_seed));
    if(result_size < 0) {
        result = (int)result_size;
        signal_log(chain_key->global_context, SG_LOG_WARNING, "ratchet_chain_key_get_base_material failed");
        goto complete;
    }
    next_key_len = (size_t)result_size;

    result = ratchet_chain_key_create(
            next_chain_key,
            chain_key->kdf,
            next_key, next_key_len,
            chain_key->index + 1,
            chain_key->global_context);

complete:
    if(next_key) {
        free(next_key);
    }

    return result;
}

void ratchet_chain_key_destroy(signal_type_base *type)
{
    ratchet_chain_key *chain_key = (ratchet_chain_key *)type;
    SIGNAL_UNREF(chain_key->kdf);
    if(chain_key->key) {
        signal_explicit_bzero(chain_key->key, chain_key->key_len);
        free(chain_key->key);
    }
    free(chain_key);
}

int ratchet_root_key_create(ratchet_root_key **root_key, hkdf_context *kdf, const uint8_t *key, size_t key_len, signal_context *global_context)
{
    ratchet_root_key *result = 0;

    if(!kdf || !key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(ratchet_root_key));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    SIGNAL_INIT(result, ratchet_root_key_destroy);
    result->global_context = global_context;
    result->kdf = kdf;

    result->key = malloc(key_len);
    if(!result->key) {
        free(result);
        return SG_ERR_NOMEM;
    }
    memcpy(result->key, key, key_len);
    result->key_len = key_len;
    SIGNAL_REF(result->kdf);
    *root_key = result;

    return 0;
}

int ratchet_root_key_create_chain(ratchet_root_key *root_key,
        ratchet_root_key **new_root_key, ratchet_chain_key **new_chain_key,
        ec_public_key *their_ratchet_key,
        ec_private_key *our_ratchet_key_private)
{
    static const char key_info[] = "WhisperRatchet";
    int result = 0;
    ssize_t result_size = 0;
    uint8_t *shared_secret = 0;
    size_t shared_secret_len = 0;
    uint8_t *derived_secret = 0;
    ratchet_root_key *new_root_key_result = 0;
    ratchet_chain_key *new_chain_key_result = 0;

    if(!their_ratchet_key || !our_ratchet_key_private) {
        return SG_ERR_INVAL;
    }

    result = curve_calculate_agreement(&shared_secret, their_ratchet_key, our_ratchet_key_private);
    if(result < 0) {
        signal_log(root_key->global_context, SG_LOG_WARNING, "curve_calculate_agreement failed");
        goto complete;
    }
    shared_secret_len = (size_t)result;

    result_size = hkdf_derive_secrets(root_key->kdf, &derived_secret,
            shared_secret, shared_secret_len,
            root_key->key, root_key->key_len,
            (uint8_t *)key_info, sizeof(key_info) - 1,
            DERIVED_ROOT_SECRETS_SIZE);
    if(result_size < 0) {
        result = (int)result_size;
        signal_log(root_key->global_context, SG_LOG_WARNING, "hkdf_derive_secrets failed");
        goto complete;
    }
    else if(result_size != DERIVED_ROOT_SECRETS_SIZE) {
        result = SG_ERR_UNKNOWN;
        signal_log(root_key->global_context, SG_LOG_WARNING, "hkdf_derive_secrets size mismatch");
        goto complete;
    }

    result = ratchet_root_key_create(&new_root_key_result, root_key->kdf,
            derived_secret, 32,
            root_key->global_context);
    if(result < 0) {
        signal_log(root_key->global_context, SG_LOG_WARNING, "ratchet_root_key_create failed");
        goto complete;
    }

    result = ratchet_chain_key_create(&new_chain_key_result, root_key->kdf,
            derived_secret + 32, 32, 0,
            root_key->global_context);
    if(result < 0) {
        signal_log(root_key->global_context, SG_LOG_WARNING, "ratchet_chain_key_create failed");
        goto complete;
    }

complete:
    if(shared_secret) {
        free(shared_secret);
    }
    if(derived_secret) {
        free(derived_secret);
    }
    if(result < 0) {
        if(new_root_key_result) {
            SIGNAL_UNREF(new_root_key_result);
        }
        if(new_chain_key_result) {
            SIGNAL_UNREF(new_chain_key_result);
        }
        return result;
    }
    else {
        *new_root_key = new_root_key_result;
        *new_chain_key = new_chain_key_result;
        return 0;
    }
}

int ratchet_root_key_get_key(ratchet_root_key *root_key, signal_buffer **buffer)
{
    signal_buffer *buf = 0;
    uint8_t *data = 0;

    assert(root_key);
    
    buf = signal_buffer_alloc(root_key->key_len);
    if(!buf) {
        return SG_ERR_NOMEM;
    }

    data = signal_buffer_data(buf);
    memcpy(data, root_key->key, root_key->key_len);

    *buffer = buf;

    return 0;
}

int ratchet_root_key_get_key_protobuf(const ratchet_root_key *root_key, ProtobufCBinaryData *buffer)
{
    uint8_t *data = 0;

    assert(root_key);
    assert(buffer);

    data = malloc(root_key->key_len);
    if(!data) {
        return SG_ERR_NOMEM;
    }

    memcpy(data, root_key->key, root_key->key_len);

    buffer->data = data;
    buffer->len = root_key->key_len;
    return 0;
}

int ratchet_root_key_compare(const ratchet_root_key *key1, const ratchet_root_key *key2)
{
    if(key1 == key2) {
        return 0;
    }
    else if(key1 == 0 && key2 != 0) {
        return -1;
    }
    else if(key1 != 0 && key2 == 0) {
        return 1;
    }
    else {
        int kdf_compare = hkdf_compare(key1->kdf, key2->kdf);
        if(kdf_compare != 0) {
            return kdf_compare;
        }
        else if(key1->key_len < key2->key_len) {
            return -1;
        }
        else if(key1->key_len > key2->key_len) {
            return 1;
        }
        else {
            return signal_constant_memcmp(key1->key, key2->key, key1->key_len);
        }
    }
}

void ratchet_root_key_destroy(signal_type_base *type)
{
    ratchet_root_key *root_key = (ratchet_root_key *)type;
    SIGNAL_UNREF(root_key->kdf);
    if(root_key->key) {
        signal_explicit_bzero(root_key->key, root_key->key_len);
        free(root_key->key);
    }
    free(root_key);
}

int ratchet_identity_key_pair_create(
        ratchet_identity_key_pair **key_pair,
        ec_public_key *public_key,
        ec_private_key *private_key)
{
    ratchet_identity_key_pair *result = malloc(sizeof(ratchet_identity_key_pair));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    SIGNAL_INIT(result, ratchet_identity_key_pair_destroy);
    SIGNAL_REF(public_key);
    SIGNAL_REF(private_key);
    result->public_key = public_key;
    result->private_key = private_key;

    *key_pair = result;

    return 0;
}

int ratchet_identity_key_pair_serialize(signal_buffer **buffer, const ratchet_identity_key_pair *key_pair)
{
    int result = 0;
    size_t result_size = 0;
    signal_buffer *result_buf = 0;
    Textsecure__IdentityKeyPairStructure key_structure = TEXTSECURE__IDENTITY_KEY_PAIR_STRUCTURE__INIT;
    size_t len = 0;
    uint8_t *data = 0;

    if(!key_pair) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    result = ec_public_key_serialize_protobuf(&key_structure.publickey, key_pair->public_key);
    if(result < 0) {
        goto complete;
    }
    key_structure.has_publickey = 1;

    result = ec_private_key_serialize_protobuf(&key_structure.privatekey, key_pair->private_key);
    if(result < 0) {
        goto complete;
    }
    key_structure.has_privatekey = 1;

    len = textsecure__identity_key_pair_structure__get_packed_size(&key_structure);
    result_buf = signal_buffer_alloc(len);
    if(!result_buf) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_buf);
    result_size = textsecure__identity_key_pair_structure__pack(&key_structure, data);
    if(result_size != len) {
        signal_buffer_free(result_buf);
        result = SG_ERR_INVALID_PROTO_BUF;
        result_buf = 0;
        goto complete;
    }

complete:
    if(key_structure.has_publickey) {
        free(key_structure.publickey.data);
    }
    if(key_structure.has_privatekey) {
        free(key_structure.privatekey.data);
    }
    if(result >= 0) {
        result = 0;
        *buffer = result_buf;
    }
    return result;
}

int ratchet_identity_key_pair_deserialize(ratchet_identity_key_pair **key_pair, const uint8_t *data, size_t len, signal_context *global_context)
{
    int result = 0;
    ec_public_key *public_key = 0;
    ec_private_key *private_key = 0;
    ratchet_identity_key_pair *result_pair = 0;
    Textsecure__IdentityKeyPairStructure *key_structure = 0;

    key_structure = textsecure__identity_key_pair_structure__unpack(0, len, data);
    if(!key_structure) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    if(!key_structure->has_publickey || !key_structure->has_privatekey) {
        result = SG_ERR_INVALID_KEY;
        goto complete;
    }

    result = curve_decode_point(
            &public_key,
            key_structure->publickey.data,
            key_structure->publickey.len,
            global_context);
    if(result < 0) {
        goto complete;
    }

    result = curve_decode_private_point(
            &private_key,
            key_structure->privatekey.data,
            key_structure->privatekey.len,
            global_context);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_identity_key_pair_create(&result_pair,
            public_key, private_key);

complete:
    SIGNAL_UNREF(public_key);
    SIGNAL_UNREF(private_key);
    if(key_structure) {
        textsecure__identity_key_pair_structure__free_unpacked(key_structure, 0);
    }
    if(result >= 0) {
        *key_pair = result_pair;
    }
    return result;
}

ec_public_key *ratchet_identity_key_pair_get_public(const ratchet_identity_key_pair *key_pair)
{
    assert(key_pair);
    assert(key_pair->public_key);
    return key_pair->public_key;
}

ec_private_key *ratchet_identity_key_pair_get_private(const ratchet_identity_key_pair *key_pair)
{
    assert(key_pair);
    assert(key_pair->private_key);
    return key_pair->private_key;
}

void ratchet_identity_key_pair_destroy(signal_type_base *type)
{
    ratchet_identity_key_pair *key_pair = (ratchet_identity_key_pair *)type;
    SIGNAL_UNREF(key_pair->public_key);
    SIGNAL_UNREF(key_pair->private_key);
    free(key_pair);
}

struct symmetric_signal_protocol_parameters
{
    signal_type_base base;
    ratchet_identity_key_pair *our_identity_key;
    ec_key_pair *our_base_key;
    ec_key_pair *our_ratchet_key;
    ec_public_key *their_base_key;
    ec_public_key *their_ratchet_key;
    ec_public_key *their_identity_key;
};

struct alice_signal_protocol_parameters
{
    signal_type_base base;
    ratchet_identity_key_pair *our_identity_key;
    ec_key_pair *our_base_key;
    ec_public_key *their_identity_key;
    ec_public_key *their_signed_pre_key;
    ec_public_key *their_one_time_pre_key; /* optional */
    ec_public_key *their_ratchet_key;
};

struct bob_signal_protocol_parameters
{
    signal_type_base base;
    ratchet_identity_key_pair *our_identity_key;
    ec_key_pair *our_signed_pre_key;
    ec_key_pair *our_one_time_pre_key; /* optional */
    ec_key_pair *our_ratchet_key;
    ec_public_key *their_identity_key;
    ec_public_key *their_base_key;
};

int symmetric_signal_protocol_parameters_create(
        symmetric_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_base_key,
        ec_key_pair *our_ratchet_key,
        ec_public_key *their_base_key,
        ec_public_key *their_ratchet_key,
        ec_public_key *their_identity_key)
{
    symmetric_signal_protocol_parameters *result = 0;
    
    if(!our_identity_key || !our_base_key || !our_ratchet_key
            || !their_base_key || !their_ratchet_key || !their_identity_key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(symmetric_signal_protocol_parameters));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    memset(result, 0, sizeof(symmetric_signal_protocol_parameters));

    SIGNAL_INIT(result, symmetric_signal_protocol_parameters_destroy);
    SIGNAL_REF(our_identity_key);
    SIGNAL_REF(our_base_key);
    SIGNAL_REF(our_ratchet_key);
    SIGNAL_REF(their_base_key);
    SIGNAL_REF(their_ratchet_key);
    SIGNAL_REF(their_identity_key);
    result->our_identity_key = our_identity_key;
    result->our_base_key = our_base_key;
    result->our_ratchet_key = our_ratchet_key;
    result->their_base_key = their_base_key;
    result->their_ratchet_key = their_ratchet_key;
    result->their_identity_key = their_identity_key;

    *parameters = result;
    return 0;
}

ratchet_identity_key_pair *symmetric_signal_protocol_parameters_get_our_identity_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->our_identity_key;
}

ec_key_pair *symmetric_signal_protocol_parameters_get_our_base_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->our_base_key;
}

ec_key_pair *symmetric_signal_protocol_parameters_get_our_ratchet_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->our_ratchet_key;
}

ec_public_key *symmetric_signal_protocol_parameters_get_their_base_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->their_base_key;
}

ec_public_key *symmetric_signal_protocol_parameters_get_their_ratchet_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->their_ratchet_key;
}

ec_public_key *symmetric_signal_protocol_parameters_get_their_identity_key(const symmetric_signal_protocol_parameters *parameters)
{
    assert(parameters);
    return parameters->their_identity_key;
}

void symmetric_signal_protocol_parameters_destroy(signal_type_base *type)
{
    symmetric_signal_protocol_parameters *parameters = (symmetric_signal_protocol_parameters *)type;

    SIGNAL_UNREF(parameters->our_identity_key);
    SIGNAL_UNREF(parameters->our_base_key);
    SIGNAL_UNREF(parameters->our_ratchet_key);
    SIGNAL_UNREF(parameters->their_base_key);
    SIGNAL_UNREF(parameters->their_ratchet_key);
    SIGNAL_UNREF(parameters->their_identity_key);

    free(parameters);
}

int alice_signal_protocol_parameters_create(
        alice_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_base_key,
        ec_public_key *their_identity_key,
        ec_public_key *their_signed_pre_key,
        ec_public_key *their_one_time_pre_key,
        ec_public_key *their_ratchet_key)
{
    alice_signal_protocol_parameters *result = 0;

    /* Only "their_one_time_pre_key" is allowed to be null */
    if(!our_identity_key || !our_base_key || !their_identity_key
            || !their_signed_pre_key || !their_ratchet_key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(alice_signal_protocol_parameters));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    memset(result, 0, sizeof(alice_signal_protocol_parameters));

    SIGNAL_INIT(result, alice_signal_protocol_parameters_destroy);
    SIGNAL_REF(our_identity_key);
    SIGNAL_REF(our_base_key);
    SIGNAL_REF(their_identity_key);
    SIGNAL_REF(their_signed_pre_key);
    SIGNAL_REF(their_ratchet_key);
    result->our_identity_key = our_identity_key;
    result->our_base_key = our_base_key;
    result->their_identity_key = their_identity_key;
    result->their_signed_pre_key = their_signed_pre_key;
    result->their_ratchet_key = their_ratchet_key;

    if(their_one_time_pre_key) {
        SIGNAL_REF(their_one_time_pre_key);
        result->their_one_time_pre_key = their_one_time_pre_key;
    }

    *parameters = result;
    return 0;
}

void alice_signal_protocol_parameters_destroy(signal_type_base *type)
{
    alice_signal_protocol_parameters *parameters = (alice_signal_protocol_parameters *)type;

    SIGNAL_UNREF(parameters->our_identity_key);
    SIGNAL_UNREF(parameters->our_base_key);
    SIGNAL_UNREF(parameters->their_identity_key);
    SIGNAL_UNREF(parameters->their_signed_pre_key);
    SIGNAL_UNREF(parameters->their_ratchet_key);

    if(parameters->their_one_time_pre_key) {
        SIGNAL_UNREF(parameters->their_one_time_pre_key);
    }

    free(parameters);
}

int bob_signal_protocol_parameters_create(
        bob_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_signed_pre_key,
        ec_key_pair *our_one_time_pre_key,
        ec_key_pair *our_ratchet_key,
        ec_public_key *their_identity_key,
        ec_public_key *their_base_key)
{
    bob_signal_protocol_parameters *result = 0;

    /* Only "our_one_time_pre_key" is allowed to be null */
    if(!our_identity_key || !our_signed_pre_key || !our_ratchet_key
            || !their_identity_key || !their_base_key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(bob_signal_protocol_parameters));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    memset(result, 0, sizeof(bob_signal_protocol_parameters));

    SIGNAL_INIT(result, bob_signal_protocol_parameters_destroy);
    SIGNAL_REF(our_identity_key);
    SIGNAL_REF(our_signed_pre_key);
    SIGNAL_REF(our_ratchet_key);
    SIGNAL_REF(their_identity_key);
    SIGNAL_REF(their_base_key);
    result->our_identity_key = our_identity_key;
    result->our_signed_pre_key = our_signed_pre_key;
    result->our_ratchet_key = our_ratchet_key;
    result->their_identity_key = their_identity_key;
    result->their_base_key = their_base_key;

    if(our_one_time_pre_key) {
        SIGNAL_REF(our_one_time_pre_key);
        result->our_one_time_pre_key = our_one_time_pre_key;
    }

    *parameters = result;
    return 0;
}

void bob_signal_protocol_parameters_destroy(signal_type_base *type)
{
    bob_signal_protocol_parameters *parameters = (bob_signal_protocol_parameters *)type;

    SIGNAL_UNREF(parameters->our_identity_key);
    SIGNAL_UNREF(parameters->our_signed_pre_key);
    SIGNAL_UNREF(parameters->our_ratchet_key);
    SIGNAL_UNREF(parameters->their_identity_key);
    SIGNAL_UNREF(parameters->their_base_key);

    if(parameters->our_one_time_pre_key) {
        SIGNAL_UNREF(parameters->our_one_time_pre_key);
    }

    free(parameters);
}

int ratcheting_session_calculate_derived_keys(ratchet_root_key **root_key, ratchet_chain_key **chain_key,
        uint8_t *secret, size_t secret_len, signal_context *global_context)
{
    int result = 0;
    ssize_t result_size = 0;
    hkdf_context *kdf = 0;
    ratchet_root_key *root_key_result = 0;
    ratchet_chain_key *chain_key_result = 0;
    uint8_t *output = 0;
    uint8_t salt[HASH_OUTPUT_SIZE];
    static const char key_info[] = "WhisperText";

    result = hkdf_create(&kdf, 3, global_context);
    if(result < 0) {
        goto complete;
    }

    memset(salt, 0, sizeof(salt));

    result_size = hkdf_derive_secrets(kdf, &output,
            secret, secret_len,
            salt, sizeof(salt),
            (uint8_t *)key_info, sizeof(key_info) - 1, 64);
    if(result_size != 64) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = ratchet_root_key_create(&root_key_result, kdf, output, 32, global_context);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_chain_key_create(&chain_key_result, kdf, output + 32, 32, 0, global_context);
    if(result < 0) {
        goto complete;
    }

complete:
    if(kdf) {
        SIGNAL_UNREF(kdf);
    }
    if(output) {
        free(output);
    }

    if(result < 0) {
        if(root_key_result) {
            SIGNAL_UNREF(root_key_result);
        }
        if(chain_key_result) {
            SIGNAL_UNREF(chain_key_result);
        }
    }
    else {
        *root_key = root_key_result;
        *chain_key = chain_key_result;
    }

    return result;
}

int ratcheting_session_symmetric_is_alice(symmetric_signal_protocol_parameters *parameters)
{
    //FIXME Java code checks if our_base_key < their_base_key
    // This comparison may not return the same result. However, we should find
    // out whether the Java code was doing the right thing and why.
    return ec_public_key_memcmp(
            ec_key_pair_get_public(parameters->our_base_key),
            parameters->their_base_key) < 0;
}

int ratcheting_session_symmetric_initialize(
        session_state *state,
        symmetric_signal_protocol_parameters *parameters,
        signal_context *global_context)
{
    int result = 0;

    assert(state);
    assert(parameters);
    assert(global_context);

    if(ratcheting_session_symmetric_is_alice(parameters)) {
        alice_signal_protocol_parameters *alice_parameters = 0;
        result = alice_signal_protocol_parameters_create(&alice_parameters,
                parameters->our_identity_key,
                parameters->our_base_key,
                parameters->their_identity_key,
                parameters->their_base_key,
                0,
                parameters->their_ratchet_key);
        if(result >= 0) {
            result = ratcheting_session_alice_initialize(state, alice_parameters, global_context);
        }
        if(alice_parameters) {
            SIGNAL_UNREF(alice_parameters);
        }
    }
    else {
        bob_signal_protocol_parameters *bob_parameters = 0;
        result = bob_signal_protocol_parameters_create(&bob_parameters,
                parameters->our_identity_key,
                parameters->our_base_key,
                0,
                parameters->our_ratchet_key,
                parameters->their_identity_key,
                parameters->their_base_key);
        if(result >= 0) {
            result = ratcheting_session_bob_initialize(state, bob_parameters, global_context);
        }
        if(bob_parameters) {
            SIGNAL_UNREF(bob_parameters);
        }
    }
    return result;
}

int ratcheting_session_alice_initialize(
        session_state *state,
        alice_signal_protocol_parameters *parameters,
        signal_context *global_context)
{
    int result = 0;
    uint8_t *agreement = 0;
    int agreement_len = 0;
    ec_key_pair *sending_ratchet_key = 0;
    ratchet_root_key *derived_root = 0;
    ratchet_chain_key *derived_chain = 0;
    ratchet_root_key *sending_chain_root = 0;
    ratchet_chain_key *sending_chain_key = 0;
    struct vpool vp;
    uint8_t *secret = 0;
    size_t secret_len = 0;
    uint8_t discontinuity_data[32];

    assert(state);
    assert(parameters);
    assert(global_context);

    vpool_init(&vp, 1024, 0);

    result = curve_generate_key_pair(global_context, &sending_ratchet_key);
    if(result < 0) {
        goto complete;
    }

    memset(discontinuity_data, 0xFF, sizeof(discontinuity_data));
    if(!vpool_insert(&vp, vpool_get_length(&vp), discontinuity_data, sizeof(discontinuity_data))) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_signed_pre_key, parameters->our_identity_key->private_key);
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_identity_key, ec_key_pair_get_private(parameters->our_base_key));
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_signed_pre_key, ec_key_pair_get_private(parameters->our_base_key));
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    if(parameters->their_one_time_pre_key) {
        agreement_len = curve_calculate_agreement(&agreement,
                parameters->their_one_time_pre_key, ec_key_pair_get_private(parameters->our_base_key));
        if(agreement_len < 0) {
            result = agreement_len;
            goto complete;
        }
        if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
            free(agreement); agreement = 0; agreement_len = 0;
        }
        else {
            result = SG_ERR_NOMEM;
            goto complete;
        }
    }

    if(vpool_is_empty(&vp)) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    secret = vpool_get_buf(&vp);
    secret_len = vpool_get_length(&vp);

    result = ratcheting_session_calculate_derived_keys(&derived_root, &derived_chain, secret, secret_len, global_context);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_root_key_create_chain(derived_root,
            &sending_chain_root, &sending_chain_key,
            parameters->their_ratchet_key,
            ec_key_pair_get_private(sending_ratchet_key));
    if(result < 0) {
        goto complete;
    }

    result = session_state_add_receiver_chain(state, parameters->their_ratchet_key, derived_chain);
    if(result < 0) {
        goto complete;
    }

    session_state_set_session_version(state, CIPHERTEXT_CURRENT_VERSION);
    session_state_set_remote_identity_key(state, parameters->their_identity_key);
    session_state_set_local_identity_key(state, parameters->our_identity_key->public_key);
    session_state_set_sender_chain(state, sending_ratchet_key, sending_chain_key);
    session_state_set_root_key(state, sending_chain_root);

complete:
    vpool_final(&vp);
    if(agreement) {
        free(agreement);
    }
    if(sending_ratchet_key) {
        SIGNAL_UNREF(sending_ratchet_key);
    }
    if(derived_root) {
        SIGNAL_UNREF(derived_root);
    }
    if(derived_chain) {
        SIGNAL_UNREF(derived_chain);
    }
    if(sending_chain_root) {
        SIGNAL_UNREF(sending_chain_root);
    }
    if(sending_chain_key) {
        SIGNAL_UNREF(sending_chain_key);
    }

    return result;
}

int ratcheting_session_bob_initialize(
        session_state *state,
        bob_signal_protocol_parameters *parameters,
        signal_context *global_context)
{
    int result = 0;
    uint8_t *agreement = 0;
    int agreement_len = 0;
    ratchet_root_key *derived_root = 0;
    ratchet_chain_key *derived_chain = 0;
    struct vpool vp;
    uint8_t *secret = 0;
    size_t secret_len = 0;
    uint8_t discontinuity_data[32];

    assert(state);
    assert(parameters);
    assert(global_context);

    vpool_init(&vp, 1024, 0);

    memset(discontinuity_data, 0xFF, sizeof(discontinuity_data));
    if(!vpool_insert(&vp, vpool_get_length(&vp), discontinuity_data, sizeof(discontinuity_data))) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_identity_key, ec_key_pair_get_private(parameters->our_signed_pre_key));
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_base_key, parameters->our_identity_key->private_key);
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    agreement_len = curve_calculate_agreement(&agreement,
            parameters->their_base_key, ec_key_pair_get_private(parameters->our_signed_pre_key));
    if(agreement_len < 0) {
        result = agreement_len;
        goto complete;
    }
    if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
        free(agreement); agreement = 0; agreement_len = 0;
    }
    else {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    if(parameters->our_one_time_pre_key) {
        agreement_len = curve_calculate_agreement(&agreement,
                parameters->their_base_key, ec_key_pair_get_private(parameters->our_one_time_pre_key));
        if(agreement_len < 0) {
            result = agreement_len;
            goto complete;
        }
        if(vpool_insert(&vp, vpool_get_length(&vp), agreement, (size_t)agreement_len)) {
            free(agreement); agreement = 0; agreement_len = 0;
        }
        else {
            result = SG_ERR_NOMEM;
            goto complete;
        }
    }

    if(vpool_is_empty(&vp)) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    secret = vpool_get_buf(&vp);
    secret_len = vpool_get_length(&vp);

    result = ratcheting_session_calculate_derived_keys(&derived_root, &derived_chain, secret, secret_len, global_context);

complete:
    if(result >= 0) {
        session_state_set_session_version(state, CIPHERTEXT_CURRENT_VERSION);
        session_state_set_remote_identity_key(state, parameters->their_identity_key);
        session_state_set_local_identity_key(state, parameters->our_identity_key->public_key);
        session_state_set_sender_chain(state, parameters->our_ratchet_key, derived_chain);
        session_state_set_root_key(state, derived_root);
    }

    vpool_final(&vp);
    if(agreement) {
        free(agreement);
    }
    if(derived_root) {
        SIGNAL_UNREF(derived_root);
    }
    if(derived_chain) {
        SIGNAL_UNREF(derived_chain);
    }

    return result;
}
