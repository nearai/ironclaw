#include "signal_protocol.h"

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <stdarg.h>
#include <assert.h>

#include "signal_protocol_internal.h"
#include "signal_utarray.h"

#ifdef _WINDOWS
#include "Windows.h"
#include "WinBase.h"
#endif

#ifdef DEBUG_REFCOUNT
int type_ref_count = 0;
int type_unref_count = 0;
#endif

#define MIN(a,b) (((a)<(b))?(a):(b))

struct signal_protocol_store_context {
    signal_context *global_context;
    signal_protocol_session_store session_store;
    signal_protocol_pre_key_store pre_key_store;
    signal_protocol_signed_pre_key_store signed_pre_key_store;
    signal_protocol_identity_key_store identity_key_store;
    signal_protocol_sender_key_store sender_key_store;
};

void signal_type_init(signal_type_base *instance,
        void (*destroy_func)(signal_type_base *instance))
{
    instance->ref_count = 1;
    instance->destroy = destroy_func;
#ifdef DEBUG_REFCOUNT
    type_ref_count++;
#endif
}

void signal_type_ref(signal_type_base *instance)
{
#ifdef DEBUG_REFCOUNT
    type_ref_count++;
#endif
    assert(instance);
    assert(instance->ref_count > 0);
    instance->ref_count++;
}

void signal_type_unref(signal_type_base *instance)
{
    if(instance) {
#ifdef DEBUG_REFCOUNT
    type_unref_count++;
#endif
        assert(instance->ref_count > 0);
        if(instance->ref_count > 1) {
            instance->ref_count--;
        }
        else {
            instance->destroy(instance);
        }
    }
}

#ifdef DEBUG_REFCOUNT
int signal_type_ref_count(signal_type_base *instance)
{
    if(!instance) {
        return 0;
    }
    return instance->ref_count;
}
#endif

/*------------------------------------------------------------------------*/

signal_buffer *signal_buffer_alloc(size_t len)
{
    signal_buffer *buffer;
    if(len > (SIZE_MAX - sizeof(struct signal_buffer)) / sizeof(uint8_t)) {
        return 0;
    }

    buffer = malloc(sizeof(struct signal_buffer) + (sizeof(uint8_t) * len));
    if(buffer) {
        buffer->len = len;
    }
    return buffer;
}

signal_buffer *signal_buffer_create(const uint8_t *data, size_t len)
{
    signal_buffer *buffer = signal_buffer_alloc(len);
    if(!buffer) {
        return 0;
    }

    memcpy(buffer->data, data, len);
    return buffer;
}

signal_buffer *signal_buffer_copy(const signal_buffer *buffer)
{
    return signal_buffer_create(buffer->data, buffer->len);
}

signal_buffer *signal_buffer_n_copy(const signal_buffer *buffer, size_t n)
{
    size_t len = MIN(buffer->len, n);
    return signal_buffer_create(buffer->data, len);
}

signal_buffer *signal_buffer_append(signal_buffer *buffer, const uint8_t *data, size_t len)
{
    signal_buffer *tmp_buffer;
    size_t previous_size = buffer->len;
    size_t previous_alloc = sizeof(struct signal_buffer) + (sizeof(uint8_t) * previous_size);

    if(len > (SIZE_MAX - previous_alloc)) {
        return 0;
    }

    tmp_buffer = realloc(buffer, previous_alloc + (sizeof(uint8_t) * len));
    if(!tmp_buffer) {
        return 0;
    }

    memcpy(tmp_buffer->data + previous_size, data, len);
    tmp_buffer->len = previous_size + len;
    return tmp_buffer;
}

uint8_t *signal_buffer_data(signal_buffer *buffer)
{
    return buffer->data;
}

const uint8_t *signal_buffer_const_data(const signal_buffer *buffer)
{
    return buffer->data;
}

size_t signal_buffer_len(const signal_buffer *buffer)
{
    return buffer->len;
}

int signal_buffer_compare(signal_buffer *buffer1, signal_buffer *buffer2)
{
    if(buffer1 == buffer2) {
        return 0;
    }
    else if(buffer1 == 0 && buffer2 != 0) {
        return -1;
    }
    else if(buffer1 != 0 && buffer2 == 0) {
        return 1;
    }
    else {
        if(buffer1->len < buffer2->len) {
            return -1;
        }
        else if(buffer1->len > buffer2->len) {
            return 1;
        }
        else {
            return signal_constant_memcmp(buffer1->data, buffer2->data, buffer1->len);
        }
    }
}

void signal_buffer_free(signal_buffer *buffer)
{
    if(buffer) {
        free(buffer);
    }
}

void signal_buffer_bzero_free(signal_buffer *buffer)
{
    if(buffer) {
        signal_explicit_bzero(buffer->data, buffer->len);
        free(buffer);
    }
}

/*------------------------------------------------------------------------*/

struct signal_buffer_list
{
    UT_array *values;
};

signal_buffer_list *signal_buffer_list_alloc(void)
{
    int result = 0;
    signal_buffer_list *list = malloc(sizeof(signal_buffer_list));
    if(!list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    memset(list, 0, sizeof(signal_buffer_list));

    utarray_new(list->values, &ut_ptr_icd);

complete:
    if(result < 0) {
        if(list) {
            free(list);
        }
        return 0;
    }
    else {
        return list;
    }
}

signal_buffer_list *signal_buffer_list_copy(const signal_buffer_list *list)
{
    int result = 0;
    signal_buffer_list *result_list = 0;
    signal_buffer *buffer_copy = 0;
    unsigned int list_size;
    unsigned int i;

    result_list = signal_buffer_list_alloc();
    if(!result_list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    list_size = utarray_len(list->values);

    utarray_reserve(result_list->values, list_size);

    for(i = 0; i < list_size; i++) {
        signal_buffer **buffer = (signal_buffer**)utarray_eltptr(list->values, i);
        buffer_copy = signal_buffer_copy(*buffer);
        utarray_push_back(result_list->values, &buffer_copy);
        buffer_copy = 0;
    }

complete:
    if(result < 0) {
        signal_buffer_free(buffer_copy);
        signal_buffer_list_free(result_list);
        return 0;
    }
    else {
        return result_list;
    }
}

int signal_buffer_list_push_back(signal_buffer_list *list, signal_buffer *buffer)
{
    int result = 0;
    assert(list);
    utarray_push_back(list->values, &buffer);

complete:
    return result;
}

unsigned int signal_buffer_list_size(signal_buffer_list *list)
{
    assert(list);
    return utarray_len(list->values);
}

signal_buffer *signal_buffer_list_at(signal_buffer_list *list, unsigned int index)
{
    signal_buffer **value = 0;

    assert(list);
    assert(index < utarray_len(list->values));

    value = (signal_buffer**)utarray_eltptr(list->values, index);

    assert(*value);

    return *value;
}

void signal_buffer_list_free(signal_buffer_list *list)
{
    unsigned int size;
    unsigned int i;
    signal_buffer **p;
    if(list) {
        size = utarray_len(list->values);
        for (i = 0; i < size; i++) {
            p = (signal_buffer **)utarray_eltptr(list->values, i);
            signal_buffer_free(*p);
        }
        utarray_free(list->values);
        free(list);
    }
}

void signal_buffer_list_bzero_free(signal_buffer_list *list)
{
    unsigned int size;
    unsigned int i;
    signal_buffer **p;
    if(list) {
        size = utarray_len(list->values);
        for (i = 0; i < size; i++) {
            p = (signal_buffer **)utarray_eltptr(list->values, i);
            signal_buffer_bzero_free(*p);
        }
        utarray_free(list->values);
        free(list);
    }
}

/*------------------------------------------------------------------------*/

struct signal_int_list
{
    UT_array *values;
};

signal_int_list *signal_int_list_alloc()
{
    int result = 0;
    signal_int_list *list = malloc(sizeof(signal_int_list));
    if(!list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    memset(list, 0, sizeof(signal_int_list));

    utarray_new(list->values, &ut_int_icd);

complete:
    if(result < 0) {
        if(list) {
            free(list);
        }
        return 0;
    }
    else {
        return list;
    }
}

int signal_int_list_push_back(signal_int_list *list, int value)
{
    int result = 0;
    assert(list);
    utarray_push_back(list->values, &value);

complete:
    return result;
}

unsigned int signal_int_list_size(signal_int_list *list)
{
    assert(list);
    return utarray_len(list->values);
}

int signal_int_list_at(signal_int_list *list, unsigned int index)
{
    int *value = 0;

    assert(list);
    assert(index < utarray_len(list->values));

    value = (int *)utarray_eltptr(list->values, index);

    assert(value);

    return *value;
}

void signal_int_list_free(signal_int_list *list)
{
    if(list) {
        utarray_free(list->values);
        free(list);
    }
}

/*------------------------------------------------------------------------*/

int signal_context_create(signal_context **context, void *user_data)
{
    *context = malloc(sizeof(signal_context));
    if(!(*context)) {
        return SG_ERR_NOMEM;
    }
    memset(*context, 0, sizeof(signal_context));
    (*context)->user_data = user_data;
#ifdef DEBUG_REFCOUNT
    type_ref_count = 0;
    type_unref_count = 0;
#endif
    return 0;
}

int signal_context_set_crypto_provider(signal_context *context, const signal_crypto_provider *crypto_provider)
{
    assert(context);
    if(!crypto_provider
            || !crypto_provider->hmac_sha256_init_func
            || !crypto_provider->hmac_sha256_update_func
            || !crypto_provider->hmac_sha256_final_func
            || !crypto_provider->hmac_sha256_cleanup_func) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->crypto_provider), crypto_provider, sizeof(signal_crypto_provider));
    return 0;
}

int signal_context_set_locking_functions(signal_context *context,
        void (*lock)(void *user_data), void (*unlock)(void *user_data))
{
    assert(context);
    if((lock && !unlock) || (!lock && unlock)) {
        return SG_ERR_INVAL;
    }

    context->lock = lock;
    context->unlock = unlock;
    return 0;
}

int signal_context_set_log_function(signal_context *context,
        void (*log)(int level, const char *message, size_t len, void *user_data))
{
    assert(context);
    context->log = log;
    return 0;
}

void signal_context_destroy(signal_context *context)
{
#ifdef DEBUG_REFCOUNT
    fprintf(stderr, "Global REF count: %d\n", type_ref_count);
    fprintf(stderr, "Global UNREF count: %d\n", type_unref_count);
#endif
    if(context) {
        free(context);
    }
}

/*------------------------------------------------------------------------*/

int signal_crypto_random(signal_context *context, uint8_t *data, size_t len)
{
    assert(context);
    assert(context->crypto_provider.random_func);
    return context->crypto_provider.random_func(data, len, context->crypto_provider.user_data);
}

int signal_hmac_sha256_init(signal_context *context, void **hmac_context, const uint8_t *key, size_t key_len)
{
    assert(context);
    assert(context->crypto_provider.hmac_sha256_init_func);
    return context->crypto_provider.hmac_sha256_init_func(hmac_context, key, key_len, context->crypto_provider.user_data);
}

int signal_hmac_sha256_update(signal_context *context, void *hmac_context, const uint8_t *data, size_t data_len)
{
    assert(context);
    assert(context->crypto_provider.hmac_sha256_update_func);
    return context->crypto_provider.hmac_sha256_update_func(hmac_context, data, data_len, context->crypto_provider.user_data);
}

int signal_hmac_sha256_final(signal_context *context, void *hmac_context, signal_buffer **output)
{
    assert(context);
    assert(context->crypto_provider.hmac_sha256_final_func);
    return context->crypto_provider.hmac_sha256_final_func(hmac_context, output, context->crypto_provider.user_data);
}

void signal_hmac_sha256_cleanup(signal_context *context, void *hmac_context)
{
    assert(context);
    assert(context->crypto_provider.hmac_sha256_cleanup_func);
    context->crypto_provider.hmac_sha256_cleanup_func(hmac_context, context->crypto_provider.user_data);
}

int signal_sha512_digest_init(signal_context *context, void **digest_context)
{
    assert(context);
    assert(context->crypto_provider.sha512_digest_init_func);
    return context->crypto_provider.sha512_digest_init_func(digest_context, context->crypto_provider.user_data);
}

int signal_sha512_digest_update(signal_context *context, void *digest_context, const uint8_t *data, size_t data_len)
{
    assert(context);
    assert(context->crypto_provider.sha512_digest_update_func);
    return context->crypto_provider.sha512_digest_update_func(digest_context, data, data_len, context->crypto_provider.user_data);
}

int signal_sha512_digest_final(signal_context *context, void *digest_context, signal_buffer **output)
{
    assert(context);
    assert(context->crypto_provider.sha512_digest_final_func);
    return context->crypto_provider.sha512_digest_final_func(digest_context, output, context->crypto_provider.user_data);
}

void signal_sha512_digest_cleanup(signal_context *context, void *digest_context)
{
    assert(context);
    assert(context->crypto_provider.sha512_digest_cleanup_func);
    return context->crypto_provider.sha512_digest_cleanup_func(digest_context, context->crypto_provider.user_data);
}

int signal_encrypt(signal_context *context,
        signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *plaintext, size_t plaintext_len)
{
    assert(context);
    assert(context->crypto_provider.encrypt_func);
    return context->crypto_provider.encrypt_func(
            output, cipher, key, key_len, iv, iv_len,
            plaintext, plaintext_len,
            context->crypto_provider.user_data);
}

int signal_decrypt(signal_context *context,
        signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *ciphertext, size_t ciphertext_len)
{
    assert(context);
    assert(context->crypto_provider.decrypt_func);
    return context->crypto_provider.decrypt_func(
            output, cipher, key, key_len, iv, iv_len,
            ciphertext, ciphertext_len,
            context->crypto_provider.user_data);
}

void signal_lock(signal_context *context)
{
    if(context->lock) {
        context->lock(context->user_data);
    }
}

void signal_unlock(signal_context *context)
{
    if(context->unlock) {
        context->unlock(context->user_data);
    }
}

void signal_log(signal_context *context, int level, const char *format, ...)
{
    char buf[256];
    int n;
    if(context && context->log) {
        va_list args;
        va_start(args, format);
        n = vsnprintf(buf, sizeof(buf), format, args);
        va_end(args);
        if(n > 0) {
            context->log(level, buf, strlen(buf), context->user_data);
        }
    }
}

void signal_explicit_bzero(void *v, size_t n)
{
#ifdef HAVE_SECUREZEROMEMORY
    SecureZeroMemory(v, n);
#elif HAVE_MEMSET_S
    memset_s(v, n, 0, n);
#else
    volatile unsigned char  *p  =  v;
    while(n--) *p++ = 0;
#endif
}

int signal_constant_memcmp(const void *s1, const void *s2, size_t n)
{
    size_t i;
    const unsigned char *c1 = (const unsigned char *) s1;
    const unsigned char *c2 = (const unsigned char *) s2;
    unsigned char result = 0;

    for (i = 0; i < n; i++) {
        result |= c1[i] ^ c2[i];
    }

    return result;
}

void signal_protocol_str_serialize_protobuf(ProtobufCBinaryData *buffer, const char *str)
{
    assert(buffer);
    assert(str);
    buffer->data = (uint8_t *)str;
    buffer->len = strlen(str);
}

char *signal_protocol_str_deserialize_protobuf(ProtobufCBinaryData *buffer)
{
    char *str = 0;
    assert(buffer);

    str = malloc(buffer->len + 1);
    if(!str) {
        return 0;
    }

    memcpy(str, buffer->data, buffer->len);
    str[buffer->len] = '\0';

    return str;
}

/*------------------------------------------------------------------------*/

int signal_protocol_store_context_create(signal_protocol_store_context **context, signal_context *global_context)
{
    assert(global_context);
    *context = malloc(sizeof(signal_protocol_store_context));
    if(!(*context)) {
        return SG_ERR_NOMEM;
    }
    memset(*context, 0, sizeof(signal_protocol_store_context));
    (*context)->global_context = global_context;
    return 0;
}

int signal_protocol_store_context_set_session_store(signal_protocol_store_context *context, const signal_protocol_session_store *store)
{
    if(!store) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->session_store), store, sizeof(signal_protocol_session_store));
    return 0;
}

int signal_protocol_store_context_set_pre_key_store(signal_protocol_store_context *context, const signal_protocol_pre_key_store *store)
{
    if(!store) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->pre_key_store), store, sizeof(signal_protocol_pre_key_store));
    return 0;
}

int signal_protocol_store_context_set_signed_pre_key_store(signal_protocol_store_context *context, const signal_protocol_signed_pre_key_store *store)
{
    if(!store) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->signed_pre_key_store), store, sizeof(signal_protocol_signed_pre_key_store));
    return 0;
}

int signal_protocol_store_context_set_identity_key_store(signal_protocol_store_context *context, const signal_protocol_identity_key_store *store)
{
    if(!store) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->identity_key_store), store, sizeof(signal_protocol_identity_key_store));
    return 0;
}

int signal_protocol_store_context_set_sender_key_store(signal_protocol_store_context *context, const signal_protocol_sender_key_store *store)
{
    if(!store) {
        return SG_ERR_INVAL;
    }
    memcpy(&(context->sender_key_store), store, sizeof(signal_protocol_sender_key_store));
    return 0;
}

void signal_protocol_store_context_destroy(signal_protocol_store_context *context)
{
    if(context) {
        if(context->session_store.destroy_func) {
            context->session_store.destroy_func(context->session_store.user_data);
        }
        if(context->pre_key_store.destroy_func) {
            context->pre_key_store.destroy_func(context->pre_key_store.user_data);
        }
        if(context->signed_pre_key_store.destroy_func) {
            context->signed_pre_key_store.destroy_func(context->signed_pre_key_store.user_data);
        }
        if(context->identity_key_store.destroy_func) {
            context->identity_key_store.destroy_func(context->identity_key_store.user_data);
        }
        if(context->sender_key_store.destroy_func) {
            context->sender_key_store.destroy_func(context->sender_key_store.user_data);
        }
        free(context);
    }
}

/*------------------------------------------------------------------------*/

int signal_protocol_session_load_session(signal_protocol_store_context *context, session_record **record, const signal_protocol_address *address)
{
    int result = 0;
    signal_buffer *buffer = 0;
    signal_buffer *user_buffer = 0;
    session_record *result_record = 0;

    assert(context);
    assert(context->session_store.load_session_func);

    result = context->session_store.load_session_func(
            &buffer, &user_buffer, address,
            context->session_store.user_data);
    if(result < 0) {
        goto complete;
    }

    if(result == 0) {
        if(buffer) {
            result = SG_ERR_UNKNOWN;
            goto complete;
        }
        result = session_record_create(&result_record, 0, context->global_context);
    }
    else if(result == 1) {
        if(!buffer) {
            result = -1;
            goto complete;
        }
        result = session_record_deserialize(&result_record,
                signal_buffer_data(buffer), signal_buffer_len(buffer), context->global_context);
    }
    else {
        result = SG_ERR_UNKNOWN;
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    if(result >= 0) {
        if(user_buffer) {
            session_record_set_user_record(result_record, user_buffer);
        }
        *record = result_record;
    }
    else {
        signal_buffer_free(user_buffer);
    }
    return result;
}

int signal_protocol_session_get_sub_device_sessions(signal_protocol_store_context *context, signal_int_list **sessions, const char *name, size_t name_len)
{
    assert(context);
    assert(context->session_store.get_sub_device_sessions_func);

    return context->session_store.get_sub_device_sessions_func(
            sessions, name, name_len,
            context->session_store.user_data);
}

int signal_protocol_session_store_session(signal_protocol_store_context *context, const signal_protocol_address *address, session_record *record)
{
    int result = 0;
    signal_buffer *buffer = 0;
    signal_buffer *user_buffer = 0;
    uint8_t *user_buffer_data = 0;
    size_t user_buffer_len = 0;

    assert(context);
    assert(context->session_store.store_session_func);
    assert(record);

    result = session_record_serialize(&buffer, record);
    if(result < 0) {
        goto complete;
    }

    user_buffer = session_record_get_user_record(record);
    if(user_buffer) {
        user_buffer_data = signal_buffer_data(user_buffer);
        user_buffer_len = signal_buffer_len(user_buffer);
    }

    result = context->session_store.store_session_func(
            address,
            signal_buffer_data(buffer), signal_buffer_len(buffer),
            user_buffer_data, user_buffer_len,
            context->session_store.user_data);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_session_contains_session(signal_protocol_store_context *context, const signal_protocol_address *address)
{
    assert(context);
    assert(context->session_store.contains_session_func);

    return context->session_store.contains_session_func(
            address,
            context->session_store.user_data);
}

int signal_protocol_session_delete_session(signal_protocol_store_context *context, const signal_protocol_address *address)
{
    assert(context);
    assert(context->session_store.delete_session_func);

    return context->session_store.delete_session_func(
            address,
            context->session_store.user_data);
}

int signal_protocol_session_delete_all_sessions(signal_protocol_store_context *context, const char *name, size_t name_len)
{
    assert(context);
    assert(context->session_store.delete_all_sessions_func);

    return context->session_store.delete_all_sessions_func(
            name, name_len,
            context->session_store.user_data);
}

/*------------------------------------------------------------------------*/

int signal_protocol_pre_key_load_key(signal_protocol_store_context *context, session_pre_key **pre_key, uint32_t pre_key_id)
{
    int result = 0;
    signal_buffer *buffer = 0;
    session_pre_key *result_key = 0;

    assert(context);
    assert(context->pre_key_store.load_pre_key);

    result = context->pre_key_store.load_pre_key(
            &buffer, pre_key_id,
            context->pre_key_store.user_data);
    if(result < 0) {
        goto complete;
    }

    result = session_pre_key_deserialize(&result_key,
            signal_buffer_data(buffer), signal_buffer_len(buffer), context->global_context);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    if(result >= 0) {
        *pre_key = result_key;
    }
    return result;
}

int signal_protocol_pre_key_store_key(signal_protocol_store_context *context, session_pre_key *pre_key)
{
    int result = 0;
    signal_buffer *buffer = 0;
    uint32_t id = 0;

    assert(context);
    assert(context->pre_key_store.store_pre_key);
    assert(pre_key);

    id = session_pre_key_get_id(pre_key);

    result = session_pre_key_serialize(&buffer, pre_key);
    if(result < 0) {
        goto complete;
    }

    result = context->pre_key_store.store_pre_key(
            id,
            signal_buffer_data(buffer), signal_buffer_len(buffer),
            context->pre_key_store.user_data);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_pre_key_contains_key(signal_protocol_store_context *context, uint32_t pre_key_id)
{
    int result = 0;

    assert(context);
    assert(context->pre_key_store.contains_pre_key);

    result = context->pre_key_store.contains_pre_key(
            pre_key_id, context->pre_key_store.user_data);

    return result;
}

int signal_protocol_pre_key_remove_key(signal_protocol_store_context *context, uint32_t pre_key_id)
{
    int result = 0;

    assert(context);
    assert(context->pre_key_store.remove_pre_key);

    result = context->pre_key_store.remove_pre_key(
            pre_key_id, context->pre_key_store.user_data);

    return result;
}

/*------------------------------------------------------------------------*/

int signal_protocol_signed_pre_key_load_key(signal_protocol_store_context *context, session_signed_pre_key **pre_key, uint32_t signed_pre_key_id)
{
    int result = 0;
    signal_buffer *buffer = 0;
    session_signed_pre_key *result_key = 0;

    assert(context);
    assert(context->signed_pre_key_store.load_signed_pre_key);

    result = context->signed_pre_key_store.load_signed_pre_key(
            &buffer, signed_pre_key_id,
            context->signed_pre_key_store.user_data);
    if(result < 0) {
        goto complete;
    }

    result = session_signed_pre_key_deserialize(&result_key,
            signal_buffer_data(buffer), signal_buffer_len(buffer), context->global_context);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    if(result >= 0) {
        *pre_key = result_key;
    }
    return result;
}

int signal_protocol_signed_pre_key_store_key(signal_protocol_store_context *context, session_signed_pre_key *pre_key)
{
    int result = 0;
    signal_buffer *buffer = 0;
    uint32_t id = 0;

    assert(context);
    assert(context->signed_pre_key_store.store_signed_pre_key);
    assert(pre_key);

    id = session_signed_pre_key_get_id(pre_key);

    result = session_signed_pre_key_serialize(&buffer, pre_key);
    if(result < 0) {
        goto complete;
    }

    result = context->signed_pre_key_store.store_signed_pre_key(
            id,
            signal_buffer_data(buffer), signal_buffer_len(buffer),
            context->signed_pre_key_store.user_data);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_signed_pre_key_contains_key(signal_protocol_store_context *context, uint32_t signed_pre_key_id)
{
    int result = 0;

    assert(context);
    assert(context->signed_pre_key_store.contains_signed_pre_key);

    result = context->signed_pre_key_store.contains_signed_pre_key(
            signed_pre_key_id, context->signed_pre_key_store.user_data);

    return result;
}

int signal_protocol_signed_pre_key_remove_key(signal_protocol_store_context *context, uint32_t signed_pre_key_id)
{
    int result = 0;

    assert(context);
    assert(context->signed_pre_key_store.remove_signed_pre_key);

    result = context->signed_pre_key_store.remove_signed_pre_key(
            signed_pre_key_id, context->signed_pre_key_store.user_data);

    return result;
}

/*------------------------------------------------------------------------*/

int signal_protocol_identity_get_key_pair(signal_protocol_store_context *context, ratchet_identity_key_pair **key_pair)
{
    int result = 0;
    signal_buffer *public_buf = 0;
    signal_buffer *private_buf = 0;
    ec_public_key *public_key = 0;
    ec_private_key *private_key = 0;
    ratchet_identity_key_pair *result_key = 0;

    assert(context);
    assert(context->identity_key_store.get_identity_key_pair);

    result = context->identity_key_store.get_identity_key_pair(
            &public_buf, &private_buf,
            context->identity_key_store.user_data);
    if(result < 0) {
        goto complete;
    }

    result = curve_decode_point(&public_key, public_buf->data, public_buf->len, context->global_context);
    if(result < 0) {
        goto complete;
    }

    result = curve_decode_private_point(&private_key, private_buf->data, private_buf->len, context->global_context);
    if(result < 0) {
        goto complete;
    }

    result = ratchet_identity_key_pair_create(&result_key, public_key, private_key);
    if(result < 0) {
        goto complete;
    }

complete:
    if(public_buf) {
        signal_buffer_free(public_buf);
    }
    if(private_buf) {
        signal_buffer_free(private_buf);
    }
    if(public_key) {
        SIGNAL_UNREF(public_key);
    }
    if(private_key) {
        SIGNAL_UNREF(private_key);
    }
    if(result >= 0) {
        *key_pair = result_key;
    }
    return result;
}

int signal_protocol_identity_get_local_registration_id(signal_protocol_store_context *context, uint32_t *registration_id)
{
    int result = 0;

    assert(context);
    assert(context->identity_key_store.get_local_registration_id);

    result = context->identity_key_store.get_local_registration_id(
            context->identity_key_store.user_data, registration_id);

    return result;
}

int signal_protocol_identity_save_identity(signal_protocol_store_context *context, const signal_protocol_address *address, ec_public_key *identity_key)
{
    int result = 0;
    signal_buffer *buffer = 0;

    assert(context);
    assert(context->identity_key_store.save_identity);

    if(identity_key) {
        result = ec_public_key_serialize(&buffer, identity_key);
        if(result < 0) {
            goto complete;
        }

        result = context->identity_key_store.save_identity(
                address,
                signal_buffer_data(buffer),
                signal_buffer_len(buffer),
                context->identity_key_store.user_data);
    }
    else {
        result = context->identity_key_store.save_identity(
                address, 0, 0,
                context->identity_key_store.user_data);
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_identity_is_trusted_identity(signal_protocol_store_context *context, const signal_protocol_address *address, ec_public_key *identity_key)
{
    int result = 0;
    signal_buffer *buffer = 0;

    assert(context);
    assert(context->identity_key_store.is_trusted_identity);

    result = ec_public_key_serialize(&buffer, identity_key);
    if(result < 0) {
        goto complete;
    }

    result = context->identity_key_store.is_trusted_identity(
            address,
            signal_buffer_data(buffer),
            signal_buffer_len(buffer),
            context->identity_key_store.user_data);
complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_sender_key_store_key(signal_protocol_store_context *context, const signal_protocol_sender_key_name *sender_key_name, sender_key_record *record)
{
    int result = 0;
    signal_buffer *buffer = 0;
    signal_buffer *user_buffer = 0;
    uint8_t *user_buffer_data = 0;
    size_t user_buffer_len = 0;

    assert(context);
    assert(context->sender_key_store.store_sender_key);
    assert(record);

    result = sender_key_record_serialize(&buffer, record);
    if(result < 0) {
        goto complete;
    }

    user_buffer = sender_key_record_get_user_record(record);
    if(user_buffer) {
        user_buffer_data = signal_buffer_data(user_buffer);
        user_buffer_len = signal_buffer_len(user_buffer);
    }

    result = context->sender_key_store.store_sender_key(
            sender_key_name,
            signal_buffer_data(buffer), signal_buffer_len(buffer),
            user_buffer_data, user_buffer_len,
            context->sender_key_store.user_data);

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }

    return result;
}

int signal_protocol_sender_key_load_key(signal_protocol_store_context *context, sender_key_record **record, const signal_protocol_sender_key_name *sender_key_name)
{
    int result = 0;
    signal_buffer *buffer = 0;
    signal_buffer *user_buffer = 0;
    sender_key_record *result_record = 0;

    assert(context);
    assert(context->sender_key_store.load_sender_key);

    result = context->sender_key_store.load_sender_key(
            &buffer, &user_buffer, sender_key_name,
            context->sender_key_store.user_data);
    if(result < 0) {
        goto complete;
    }

    if(result == 0) {
        if(buffer) {
            result = SG_ERR_UNKNOWN;
            goto complete;
        }
        result = sender_key_record_create(&result_record, context->global_context);
    }
    else if(result == 1) {
        if(!buffer) {
            result = -1;
            goto complete;
        }
        result = sender_key_record_deserialize(&result_record,
                signal_buffer_data(buffer), signal_buffer_len(buffer), context->global_context);
    }
    else {
        result = SG_ERR_UNKNOWN;
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    if(result >= 0) {
        if(user_buffer) {
            sender_key_record_set_user_record(result_record, user_buffer);
        }
        *record = result_record;
    }
    else {
        signal_buffer_free(user_buffer);
    }
    return result;
}
