#include "sender_key.h"

#include <assert.h>
#include <string.h>

#include "hkdf.h"
#include "signal_protocol_internal.h"

#define HASH_OUTPUT_SIZE 32

struct sender_message_key {
    signal_type_base base;
    uint32_t iteration;
    signal_buffer *iv;
    signal_buffer *cipher_key;
    signal_buffer *seed;
    signal_context *global_context;
};

struct sender_chain_key {
    signal_type_base base;
    uint32_t iteration;
    signal_buffer *chain_key;
    signal_context *global_context;
};

static int sender_chain_key_get_derivative(signal_buffer **derivative, uint8_t seed, signal_buffer *key,
        signal_context *global_context);

int sender_message_key_create(sender_message_key **key,
        uint32_t iteration, signal_buffer *seed,
        signal_context *global_context)
{
    sender_message_key *result = 0;
    int ret = 0;
    ssize_t ret_size = 0;
    hkdf_context *kdf = 0;
    static const char info_material[] = "WhisperGroup";
    uint8_t salt[HASH_OUTPUT_SIZE];
    uint8_t *derivative = 0;

    assert(global_context);

    if(!seed) {
        return SG_ERR_INVAL;
    }

    memset(salt, 0, sizeof(salt));

    result = malloc(sizeof(sender_message_key));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    SIGNAL_INIT(result, sender_message_key_destroy);

    ret = hkdf_create(&kdf, 3, global_context);
    if(ret < 0) {
        goto complete;
    }

    ret_size = hkdf_derive_secrets(kdf, &derivative,
            signal_buffer_data(seed), signal_buffer_len(seed),
            salt, sizeof(salt),
            (uint8_t *)info_material, sizeof(info_material) - 1, 48);
    if(ret_size != 48) {
        ret = (ret_size < 0) ? (int)ret_size : SG_ERR_UNKNOWN;
        signal_log(global_context, SG_LOG_WARNING, "hkdf_derive_secrets failed");
        goto complete;
    }

    result->iteration = iteration;

    result->seed = signal_buffer_copy(seed);
    if(!result->seed) {
        ret = SG_ERR_NOMEM;
        goto complete;
    }

    result->iv = signal_buffer_create(derivative, 16);
    if(!result->iv) {
        ret = SG_ERR_NOMEM;
        goto complete;
    }

    result->cipher_key = signal_buffer_create(derivative + 16, 32);
    if(!result->cipher_key) {
        ret = SG_ERR_NOMEM;
        goto complete;
    }

    result->global_context = global_context;

complete:
    SIGNAL_UNREF(kdf);
    if(derivative) {
        free(derivative);
    }
    if(ret < 0) {
        SIGNAL_UNREF(result);
    }
    else {
        ret = 0;
        *key = result;
    }
    return ret;
}

uint32_t sender_message_key_get_iteration(sender_message_key *key)
{
    assert(key);
    return key->iteration;
}

signal_buffer *sender_message_key_get_iv(sender_message_key *key)
{
    assert(key);
    return key->iv;
}

signal_buffer *sender_message_key_get_cipher_key(sender_message_key *key)
{
    assert(key);
    return key->cipher_key;
}

signal_buffer *sender_message_key_get_seed(sender_message_key *key)
{
    assert(key);
    return key->seed;
}

void sender_message_key_destroy(signal_type_base *type)
{
    sender_message_key *key = (sender_message_key *)type;
    signal_buffer_bzero_free(key->iv);
    signal_buffer_bzero_free(key->cipher_key);
    signal_buffer_bzero_free(key->seed);
    free(key);
}

int sender_chain_key_create(sender_chain_key **key,
        uint32_t iteration, signal_buffer *chain_key,
        signal_context *global_context)
{
    sender_chain_key *result = 0;
    int ret = 0;

    assert(global_context);

    if(!chain_key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(sender_chain_key));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    SIGNAL_INIT(result, sender_chain_key_destroy);

    result->iteration = iteration;

    result->chain_key = signal_buffer_copy(chain_key);
    if(!result->chain_key) {
        ret = SG_ERR_NOMEM;
        goto complete;
    }

    result->global_context = global_context;

complete:
    if(ret < 0) {
        SIGNAL_UNREF(result);
    }
    else {
        ret = 0;
        *key = result;
    }
    return ret;
}

uint32_t sender_chain_key_get_iteration(sender_chain_key *key)
{
    assert(key);
    return key->iteration;
}

int sender_chain_key_create_message_key(sender_chain_key *key, sender_message_key **message_key)
{
    static const uint8_t MESSAGE_KEY_SEED = 0x01;
    int ret = 0;
    signal_buffer *derivative = 0;
    sender_message_key *result = 0;

    assert(key);

    ret = sender_chain_key_get_derivative(&derivative, MESSAGE_KEY_SEED, key->chain_key, key->global_context);
    if(ret < 0) {
        goto complete;
    }

    ret = sender_message_key_create(&result, key->iteration, derivative, key->global_context);

complete:
    signal_buffer_free(derivative);
    if(ret >= 0) {
        ret = 0;
        *message_key = result;
    }
    return ret;
}

int sender_chain_key_create_next(sender_chain_key *key, sender_chain_key **next_key)
{
    static const uint8_t CHAIN_KEY_SEED = 0x02;
    int ret = 0;
    signal_buffer *derivative = 0;
    sender_chain_key *result = 0;

    assert(key);

    ret = sender_chain_key_get_derivative(&derivative, CHAIN_KEY_SEED, key->chain_key, key->global_context);
    if(ret < 0) {
        goto complete;
    }

    ret = sender_chain_key_create(&result, key->iteration + 1, derivative, key->global_context);

complete:
    signal_buffer_free(derivative);
    if(ret >= 0) {
        ret = 0;
        *next_key = result;
    }
    return ret;
}

signal_buffer *sender_chain_key_get_seed(sender_chain_key *key)
{
    assert(key);
    return key->chain_key;
}

void sender_chain_key_destroy(signal_type_base *type)
{
    sender_chain_key *key = (sender_chain_key *)type;
    signal_buffer_bzero_free(key->chain_key);
    free(key);
}

int sender_chain_key_get_derivative(signal_buffer **derivative, uint8_t seed, signal_buffer *key,
        signal_context *global_context)
{
    int result = 0;
    signal_buffer *output_buffer = 0;
    void *hmac_context = 0;

    result = signal_hmac_sha256_init(global_context, &hmac_context,
            signal_buffer_data(key), signal_buffer_len(key));
    if(result < 0) {
        goto complete;
    }

    result = signal_hmac_sha256_update(global_context, hmac_context, &seed, sizeof(seed));
    if(result < 0) {
        goto complete;
    }

    result = signal_hmac_sha256_final(global_context, hmac_context, &output_buffer);
    if(result < 0) {
        goto complete;
    }

complete:
    signal_hmac_sha256_cleanup(global_context, hmac_context);

    if(result < 0) {
        signal_buffer_free(output_buffer);
    }
    else {
        *derivative = output_buffer;
    }
    return result;
}
