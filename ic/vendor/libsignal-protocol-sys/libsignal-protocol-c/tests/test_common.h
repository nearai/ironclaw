#ifndef TEST_COMMON_H
#define TEST_COMMON_H

#include <stdint.h>
#include <stddef.h>
#include "../src/signal_protocol.h"

/* Test utility functions */
void print_public_key(const char *prefix, ec_public_key *key);
void print_buffer(const char *prefix, signal_buffer *buffer);
void shuffle_buffers(signal_buffer **array, size_t n);
void shuffle_ec_public_keys(ec_public_key **array, size_t n);
ec_public_key *create_test_ec_public_key(signal_context *context);
ec_private_key *create_test_ec_private_key(signal_context *context);

/* Test logging */
void test_log(int level, const char *message, size_t len, void *user_data);

/* Test crypto provider */
int test_random_generator(uint8_t *data, size_t len, void *user_data);
int test_hmac_sha256_init(void **hmac_context, const uint8_t *key, size_t key_len, void *user_data);
int test_hmac_sha256_update(void *hmac_context, const uint8_t *data, size_t data_len, void *user_data);
int test_hmac_sha256_final(void *hmac_context, signal_buffer **output, void *user_data);
void test_hmac_sha256_cleanup(void *hmac_context, void *user_data);
int test_sha512_digest_init(void **digest_context, void *user_data);
int test_sha512_digest_update(void *digest_context, const uint8_t *data, size_t data_len, void *user_data);
int test_sha512_digest_final(void *digest_context, signal_buffer **output, void *user_data);
void test_sha512_digest_cleanup(void *digest_context, void *user_data);

int test_encrypt(signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *plaintext, size_t plaintext_len,
        void *user_data);
int test_decrypt(signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *ciphertext, size_t ciphertext_len,
        void *user_data);
void setup_test_crypto_provider(signal_context *context);

/* Test data store context */
void setup_test_store_context(signal_protocol_store_context **context, signal_context *global_context);

/* Test session store */
int test_session_store_load_session(signal_buffer **record, signal_buffer **user_record, const signal_protocol_address *address, void *user_data);
int test_session_store_get_sub_device_sessions(signal_int_list **sessions, const char *name, size_t name_len, void *user_data);
int test_session_store_store_session(const signal_protocol_address *address, uint8_t *record, size_t record_len, uint8_t *user_record_data, size_t user_record_len, void *user_data);
int test_session_store_contains_session(const signal_protocol_address *address, void *user_data);
int test_session_store_delete_session(const signal_protocol_address *address, void *user_data);
int test_session_store_delete_all_sessions(const char *name, size_t name_len, void *user_data);
void test_session_store_destroy(void *user_data);
void setup_test_session_store(signal_protocol_store_context *context);

/* Test pre-key store */
int test_pre_key_store_load_pre_key(signal_buffer **record, uint32_t pre_key_id, void *user_data);
int test_pre_key_store_store_pre_key(uint32_t pre_key_id, uint8_t *record, size_t record_len, void *user_data);
int test_pre_key_store_contains_pre_key(uint32_t pre_key_id, void *user_data);
int test_pre_key_store_remove_pre_key(uint32_t pre_key_id, void *user_data);
void test_pre_key_store_destroy(void *user_data);
void setup_test_pre_key_store(signal_protocol_store_context *context);

/* Test signed pre-key store */
int test_signed_pre_key_store_load_signed_pre_key(signal_buffer **record, uint32_t signed_pre_key_id, void *user_data);
int test_signed_pre_key_store_store_signed_pre_key(uint32_t signed_pre_key_id, uint8_t *record, size_t record_len, void *user_data);
int test_signed_pre_key_store_contains_signed_pre_key(uint32_t signed_pre_key_id, void *user_data);
int test_signed_pre_key_store_remove_signed_pre_key(uint32_t signed_pre_key_id, void *user_data);
void test_signed_pre_key_store_destroy(void *user_data);
void setup_test_signed_pre_key_store(signal_protocol_store_context *context);

/* Test identity key store */
int test_identity_key_store_get_identity_key_pair(signal_buffer **public_data, signal_buffer **private_data, void *user_data);
int test_identity_key_store_get_local_registration_id(void *user_data, uint32_t *registration_id);
int test_identity_key_store_save_identity(const signal_protocol_address *address, uint8_t *key_data, size_t key_len, void *user_data);
int test_identity_key_store_is_trusted_identity(const signal_protocol_address *address, uint8_t *key_data, size_t key_len, void *user_data);
void test_identity_key_store_destroy(void *user_data);
void setup_test_identity_key_store(signal_protocol_store_context *context, signal_context *global_context);

/* Test sender key store */
int test_sender_key_store_store_sender_key(const signal_protocol_sender_key_name *sender_key_name, uint8_t *record, size_t record_len, uint8_t *user_record_data, size_t user_record_len, void *user_data);
int test_sender_key_store_load_sender_key(signal_buffer **record, signal_buffer **user_record, const signal_protocol_sender_key_name *sender_key_name, void *user_data);
void test_sender_key_store_destroy(void *user_data);
void setup_test_sender_key_store(signal_protocol_store_context *context, signal_context *global_context);

/* Portability */
#ifndef __OpenBSD__
/* OpenBSD extension */
void srand_deterministic(unsigned int seed);
#endif

#endif /* TEST_COMMON_H */
