#include <stdio.h>
#include <check.h>

#include "../src/signal_protocol.h"
#include "hkdf.h"
#include "ratchet.h"
#include "session_state.h"
#include "test_common.h"

signal_context *global_context;

void test_setup()
{
    int result;
    result = signal_context_create(&global_context, 0);
    ck_assert_int_eq(result, 0);
    signal_context_set_log_function(global_context, test_log);

    setup_test_crypto_provider(global_context);
}

void test_teardown()
{
    signal_context_destroy(global_context);
}

static uint8_t seed[] = {
    0x8a, 0xb7, 0x2d, 0x6f, 0x4c, 0xc5, 0xac, 0x0d,
    0x38, 0x7e, 0xaf, 0x46, 0x33, 0x78, 0xdd, 0xb2,
    0x8e, 0xdd, 0x07, 0x38, 0x5b, 0x1c, 0xb0, 0x12,
    0x50, 0xc7, 0x15, 0x98, 0x2e, 0x7a, 0xd4, 0x8f};

START_TEST(test_chain_key_derivation_v2)
{
    int result = 0;

    uint8_t messageKey[] = {
            0x02, 0xa9, 0xaa, 0x6c, 0x7d, 0xbd, 0x64, 0xf9,
            0xd3, 0xaa, 0x92, 0xf9, 0x2a, 0x27, 0x7b, 0xf5,
            0x46, 0x09, 0xda, 0xdf, 0x0b, 0x00, 0x82, 0x8a,
            0xcf, 0xc6, 0x1e, 0x3c, 0x72, 0x4b, 0x84, 0xa7};

    uint8_t macKey[] = {
            0xbf, 0xbe, 0x5e, 0xfb, 0x60, 0x30, 0x30, 0x52,
            0x67, 0x42, 0xe3, 0xee, 0x89, 0xc7, 0x02, 0x4e,
            0x88, 0x4e, 0x44, 0x0f, 0x1f, 0xf3, 0x76, 0xbb,
            0x23, 0x17, 0xb2, 0xd6, 0x4d, 0xeb, 0x7c, 0x83};

    uint8_t nextChainKey[] = {
            0x28, 0xe8, 0xf8, 0xfe, 0xe5, 0x4b, 0x80, 0x1e,
            0xef, 0x7c, 0x5c, 0xfb, 0x2f, 0x17, 0xf3, 0x2c,
            0x7b, 0x33, 0x44, 0x85, 0xbb, 0xb7, 0x0f, 0xac,
            0x6e, 0xc1, 0x03, 0x42, 0xa2, 0x46, 0xd1, 0x5d};

    hkdf_context *kdf;
    result = hkdf_create(&kdf, 2, global_context);
    ck_assert_int_eq(result, 0);

    ratchet_chain_key *chain_key;
    result = ratchet_chain_key_create(&chain_key, kdf, seed, sizeof(seed), 0, global_context);
    ck_assert_int_eq(result, 0);

    SIGNAL_UNREF(kdf);

    signal_buffer *actual_key;
    result = ratchet_chain_key_get_key(chain_key, &actual_key);
    ck_assert_int_eq(result, 0);
    int actual_key_len = signal_buffer_len(actual_key);
    ck_assert_int_eq(actual_key_len, sizeof(seed));
    ck_assert_int_eq(memcmp(signal_buffer_data(actual_key), seed, actual_key_len), 0);
    signal_buffer_free(actual_key);

    ratchet_message_keys message_keys;
    result = ratchet_chain_key_get_message_keys(chain_key, &message_keys);
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(memcmp(message_keys.cipher_key, messageKey, sizeof(messageKey)), 0);
    ck_assert_int_eq(memcmp(message_keys.mac_key, macKey, sizeof(macKey)), 0);
    ck_assert_int_eq(message_keys.counter, 0);

    ratchet_chain_key *next_chain_key;
    result = ratchet_chain_key_create_next(chain_key, &next_chain_key);
    ck_assert_int_eq(result, 0);

    result = ratchet_chain_key_get_key(next_chain_key, &actual_key);
    ck_assert_int_eq(result, 0);
    actual_key_len = signal_buffer_len(actual_key);
    ck_assert_int_eq(actual_key_len, sizeof(nextChainKey));
    ck_assert_int_eq(memcmp(signal_buffer_data(actual_key), nextChainKey, actual_key_len), 0);
    signal_buffer_free(actual_key);

    ck_assert_int_eq(ratchet_chain_key_get_index(chain_key), 0);
    ck_assert_int_eq(ratchet_chain_key_get_index(next_chain_key), 1);

    ratchet_message_keys next_message_keys;
    result = ratchet_chain_key_get_message_keys(next_chain_key, &next_message_keys);
    ck_assert_int_eq(result, 0);
    ck_assert_int_eq(next_message_keys.counter, 1);

    SIGNAL_UNREF(chain_key);
    SIGNAL_UNREF(next_chain_key);
}
END_TEST

START_TEST(test_chain_key_derivation_v3)
{
    int result = 0;

    uint8_t messageKey[] = {
            0xbf, 0x51, 0xe9, 0xd7, 0x5e, 0x0e, 0x31, 0x03,
            0x10, 0x51, 0xf8, 0x2a, 0x24, 0x91, 0xff, 0xc0,
            0x84, 0xfa, 0x29, 0x8b, 0x77, 0x93, 0xbd, 0x9d,
            0xb6, 0x20, 0x05, 0x6f, 0xeb, 0xf4, 0x52, 0x17};

    uint8_t macKey[] = {
            0xc6, 0xc7, 0x7d, 0x6a, 0x73, 0xa3, 0x54, 0x33,
            0x7a, 0x56, 0x43, 0x5e, 0x34, 0x60, 0x7d, 0xfe,
            0x48, 0xe3, 0xac, 0xe1, 0x4e, 0x77, 0x31, 0x4d,
            0xc6, 0xab, 0xc1, 0x72, 0xe7, 0xa7, 0x03, 0x0b};

    uint8_t nextChainKey[] = {
            0x28, 0xe8, 0xf8, 0xfe, 0xe5, 0x4b, 0x80, 0x1e,
            0xef, 0x7c, 0x5c, 0xfb, 0x2f, 0x17, 0xf3, 0x2c,
            0x7b, 0x33, 0x44, 0x85, 0xbb, 0xb7, 0x0f, 0xac,
            0x6e, 0xc1, 0x03, 0x42, 0xa2, 0x46, 0xd1, 0x5d};

    hkdf_context *kdf;
    result = hkdf_create(&kdf, 3, global_context);
    ck_assert_int_eq(result, 0);

    ratchet_chain_key *chain_key;
    result = ratchet_chain_key_create(&chain_key, kdf, seed, sizeof(seed), 0, global_context);
    ck_assert_int_eq(result, 0);

    SIGNAL_UNREF(kdf);

    signal_buffer *actual_key;
    result = ratchet_chain_key_get_key(chain_key, &actual_key);
    ck_assert_int_eq(result, 0);
    int actual_key_len = signal_buffer_len(actual_key);
    ck_assert_int_eq(actual_key_len, sizeof(seed));
    ck_assert_int_eq(memcmp(signal_buffer_data(actual_key), seed, actual_key_len), 0);
    signal_buffer_free(actual_key);

    ratchet_message_keys message_keys;
    result = ratchet_chain_key_get_message_keys(chain_key, &message_keys);
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(memcmp(message_keys.cipher_key, messageKey, sizeof(messageKey)), 0);
    ck_assert_int_eq(memcmp(message_keys.mac_key, macKey, sizeof(macKey)), 0);
    ck_assert_int_eq(message_keys.counter, 0);

    ratchet_chain_key *next_chain_key;
    result = ratchet_chain_key_create_next(chain_key, &next_chain_key);
    ck_assert_int_eq(result, 0);

    result = ratchet_chain_key_get_key(next_chain_key, &actual_key);
    ck_assert_int_eq(result, 0);
    actual_key_len = signal_buffer_len(actual_key);
    ck_assert_int_eq(actual_key_len, sizeof(nextChainKey));
    ck_assert_int_eq(memcmp(signal_buffer_data(actual_key), nextChainKey, actual_key_len), 0);
    signal_buffer_free(actual_key);

    ck_assert_int_eq(ratchet_chain_key_get_index(chain_key), 0);
    ck_assert_int_eq(ratchet_chain_key_get_index(next_chain_key), 1);

    ratchet_message_keys next_message_keys;
    result = ratchet_chain_key_get_message_keys(next_chain_key, &next_message_keys);
    ck_assert_int_eq(result, 0);
    ck_assert_int_eq(next_message_keys.counter, 1);

    SIGNAL_UNREF(chain_key);
    SIGNAL_UNREF(next_chain_key);
}
END_TEST

START_TEST(test_root_key_derivation_v2)
{
    int result = 0;
    uint8_t rootKeySeed[] = {
            0x7b, 0xa6, 0xde, 0xbc, 0x2b, 0xc1, 0xbb, 0xf9,
            0x1a, 0xbb, 0xc1, 0x36, 0x74, 0x04, 0x17, 0x6c,
            0xa6, 0x23, 0x09, 0x5b, 0x7e, 0xc6, 0x6b, 0x45,
            0xf6, 0x02, 0xd9, 0x35, 0x38, 0x94, 0x2d, 0xcc};

    uint8_t alicePrivate[] = {
            0x21, 0x68, 0x22, 0xec, 0x67, 0xeb, 0x38, 0x04,
            0x9e, 0xba, 0xe7, 0xb9, 0x39, 0xba, 0xea, 0xeb,
            0xb1, 0x51, 0xbb, 0xb3, 0x2d, 0xb8, 0x0f, 0xd3,
            0x89, 0x24, 0x5a, 0xc3, 0x7a, 0x94, 0x8e, 0x50};

    uint8_t bobPublic[] = {
            0x05, 0xab, 0xb8, 0xeb, 0x29, 0xcc, 0x80, 0xb4,
            0x71, 0x09, 0xa2, 0x26, 0x5a, 0xbe, 0x97, 0x98,
            0x48, 0x54, 0x06, 0xe3, 0x2d, 0xa2, 0x68, 0x93,
            0x4a, 0x95, 0x55, 0xe8, 0x47, 0x57, 0x70, 0x8a,
            0x30};

    uint8_t nextRoot[] = {
            0xb1, 0x14, 0xf5, 0xde, 0x28, 0x01, 0x19, 0x85,
            0xe6, 0xeb, 0xa2, 0x5d, 0x50, 0xe7, 0xec, 0x41,
            0xa9, 0xb0, 0x2f, 0x56, 0x93, 0xc5, 0xc7, 0x88,
            0xa6, 0x3a, 0x06, 0xd2, 0x12, 0xa2, 0xf7, 0x31};

    uint8_t nextChain[] = {
            0x9d, 0x7d, 0x24, 0x69, 0xbc, 0x9a, 0xe5, 0x3e,
            0xe9, 0x80, 0x5a, 0xa3, 0x26, 0x4d, 0x24, 0x99,
            0xa3, 0xac, 0xe8, 0x0f, 0x4c, 0xca, 0xe2, 0xda,
            0x13, 0x43, 0x0c, 0x5c, 0x55, 0xb5, 0xca, 0x5f};

    ec_private_key *alice_private_key = 0;
    ec_public_key *bob_public_key = 0;

    /* Initialize Alice's private key */
    result = curve_decode_private_point(&alice_private_key, alicePrivate, sizeof(alicePrivate), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_private_key, 0);

    /* Initialize Bob's public key */
    result = curve_decode_point(&bob_public_key, bobPublic, sizeof(bobPublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(bob_public_key, 0);

    hkdf_context *kdf;
    result = hkdf_create(&kdf, 2, global_context);
    ck_assert_int_eq(result, 0);

    ratchet_root_key *root_key;
    result = ratchet_root_key_create(&root_key, kdf, rootKeySeed, sizeof(rootKeySeed), global_context);
    ck_assert_int_eq(result, 0);

    SIGNAL_UNREF(kdf);

    /* Get the next key pair in the chain */
    ratchet_root_key *next_root_key;
    ratchet_chain_key *next_chain_key;
    result = ratchet_root_key_create_chain(root_key,
            &next_root_key, &next_chain_key,
            bob_public_key, alice_private_key);
    ck_assert_int_eq(result, 0);

    /* Check the value of the root key */
    signal_buffer *root_bytes;
    result = ratchet_root_key_get_key(root_key, &root_bytes);
    ck_assert_int_eq(result, 0);
    int root_len = signal_buffer_len(root_bytes);
    ck_assert_int_eq(root_len, sizeof(rootKeySeed));

    ck_assert_int_eq(memcmp(signal_buffer_data(root_bytes), rootKeySeed, root_len), 0);
    signal_buffer_free(root_bytes);

    /* Check the value of the next root key */
    signal_buffer *next_root_bytes;
    result = ratchet_root_key_get_key(next_root_key, &next_root_bytes);
    ck_assert_int_eq(result, 0);
    int next_root_len = signal_buffer_len(next_root_bytes);
    ck_assert_int_eq(next_root_len, sizeof(nextRoot));
    ck_assert_int_eq(memcmp(signal_buffer_data(next_root_bytes), nextRoot, next_root_len), 0);
    signal_buffer_free(next_root_bytes);

    /* Check the value of the next chain key */
    signal_buffer *next_chain_bytes;
    result = ratchet_chain_key_get_key(next_chain_key, &next_chain_bytes);
    ck_assert_int_eq(result, 0);
    int next_chain_len = signal_buffer_len(next_chain_bytes);
    ck_assert_int_eq(next_chain_len, sizeof(nextChain));
    ck_assert_int_eq(memcmp(signal_buffer_data(next_chain_bytes), nextChain, next_chain_len), 0);
    signal_buffer_free(next_chain_bytes);

    /* Cleanup */
    SIGNAL_UNREF(next_root_key);
    SIGNAL_UNREF(next_chain_key);
    SIGNAL_UNREF(root_key);
    SIGNAL_UNREF(alice_private_key);
    SIGNAL_UNREF(bob_public_key);
}
END_TEST

START_TEST(test_identity_key_serialize)
{
    int result = 0;
    ec_key_pair *key_pair = 0;
    ratchet_identity_key_pair *identity_key_pair = 0;
    ratchet_identity_key_pair *result_identity_key_pair = 0;
    signal_buffer *buffer = 0;

    result = curve_generate_key_pair(global_context, &key_pair);
    ck_assert_int_eq(result, 0);

    ec_public_key *public_key = ec_key_pair_get_public(key_pair);
    ec_private_key *private_key = ec_key_pair_get_private(key_pair);

    result = ratchet_identity_key_pair_create(
            &identity_key_pair, public_key, private_key);
    ck_assert_int_eq(result, 0);

    result = ratchet_identity_key_pair_serialize(&buffer, identity_key_pair);
    ck_assert_int_ge(result, 0);

    result = ratchet_identity_key_pair_deserialize(&result_identity_key_pair,
            signal_buffer_data(buffer), signal_buffer_len(buffer), global_context);
    ck_assert_int_eq(result, 0);

    ec_public_key *result_public_key = ratchet_identity_key_pair_get_public(result_identity_key_pair);
    ck_assert_int_eq(ec_public_key_compare(public_key, result_public_key), 0);

    ec_private_key *result_private_key = ratchet_identity_key_pair_get_private(result_identity_key_pair);
    ck_assert_int_eq(ec_private_key_compare(private_key, result_private_key), 0);

    /* Cleanup */
    SIGNAL_UNREF(key_pair);
    SIGNAL_UNREF(identity_key_pair);
    SIGNAL_UNREF(result_identity_key_pair);
    signal_buffer_free(buffer);
}
END_TEST

static uint8_t bobPublic[] = {
    0x05, 0x2c, 0xb4, 0x97, 0x76, 0xb8, 0x77, 0x02,
    0x05, 0x74, 0x5a, 0x3a, 0x6e, 0x24, 0xf5, 0x79,
    0xcd, 0xb4, 0xba, 0x7a, 0x89, 0x04, 0x10, 0x05,
    0x92, 0x8e, 0xbb, 0xad, 0xc9, 0xc0, 0x5a, 0xd4,
    0x58};

static uint8_t bobIdentityPublic[] = {
    0x05, 0xf1, 0xf4, 0x38, 0x74, 0xf6, 0x96, 0x69,
    0x56, 0xc2, 0xdd, 0x47, 0x3f, 0x8f, 0xa1, 0x5a,
    0xde, 0xb7, 0x1d, 0x1c, 0xb9, 0x91, 0xb2, 0x34,
    0x16, 0x92, 0x32, 0x4c, 0xef, 0xb1, 0xc5, 0xe6,
    0x26};

static uint8_t aliceBasePublic[] = {
    0x05, 0x47, 0x2d, 0x1f, 0xb1, 0xa9, 0x86, 0x2c,
    0x3a, 0xf6, 0xbe, 0xac, 0xa8, 0x92, 0x02, 0x77,
    0xe2, 0xb2, 0x6f, 0x4a, 0x79, 0x21, 0x3e, 0xc7,
    0xc9, 0x06, 0xae, 0xb3, 0x5e, 0x03, 0xcf, 0x89,
    0x50};

static uint8_t aliceIdentityPublic[] = {
    0x05, 0xb4, 0xa8, 0x45, 0x56, 0x60, 0xad, 0xa6,
    0x5b, 0x40, 0x10, 0x07, 0xf6, 0x15, 0xe6, 0x54,
    0x04, 0x17, 0x46, 0x43, 0x2e, 0x33, 0x39, 0xc6,
    0x87, 0x51, 0x49, 0xbc, 0xee, 0xfc, 0xb4, 0x2b,
    0x4a};

static uint8_t bobSignedPreKeyPublic[] = {
    0x05, 0xac, 0x24, 0x8a, 0x8f, 0x26, 0x3b, 0xe6,
    0x86, 0x35, 0x76, 0xeb, 0x03, 0x62, 0xe2, 0x8c,
    0x82, 0x8f, 0x01, 0x07, 0xa3, 0x37, 0x9d, 0x34,
    0xba, 0xb1, 0x58, 0x6b, 0xf8, 0xc7, 0x70, 0xcd,
    0x67};

static uint8_t receiverAndSenderChain[] = {
    0x97, 0x97, 0xca, 0xca, 0x53, 0xc9, 0x89, 0xbb,
    0xe2, 0x29, 0xa4, 0x0c, 0xa7, 0x72, 0x70, 0x10,
    0xeb, 0x26, 0x04, 0xfc, 0x14, 0x94, 0x5d, 0x77,
    0x95, 0x8a, 0x0a, 0xed, 0xa0, 0x88, 0xb4, 0x4d};

START_TEST(test_ratcheting_session_as_bob)
{
    int result = 0;

    uint8_t bobPrivate[] = {
            0xa1, 0xca, 0xb4, 0x8f, 0x7c, 0x89, 0x3f, 0xaf,
            0xa9, 0x88, 0x0a, 0x28, 0xc3, 0xb4, 0x99, 0x9d,
            0x28, 0xd6, 0x32, 0x95, 0x62, 0xd2, 0x7a, 0x4e,
            0xa4, 0xe2, 0x2e, 0x9f, 0xf1, 0xbd, 0xd6, 0x5a};

    uint8_t bobIdentityPrivate[] = {
            0x48, 0x75, 0xcc, 0x69, 0xdd, 0xf8, 0xea, 0x07,
            0x19, 0xec, 0x94, 0x7d, 0x61, 0x08, 0x11, 0x35,
            0x86, 0x8d, 0x5f, 0xd8, 0x01, 0xf0, 0x2c, 0x02,
            0x25, 0xe5, 0x16, 0xdf, 0x21, 0x56, 0x60, 0x5e};

    uint8_t bobSignedPreKeyPrivate[] = {
            0x58, 0x39, 0x00, 0x13, 0x1f, 0xb7, 0x27, 0x99,
            0x8b, 0x78, 0x03, 0xfe, 0x6a, 0xc2, 0x2c, 0xc5,
            0x91, 0xf3, 0x42, 0xe4, 0xe4, 0x2a, 0x8c, 0x8d,
            0x5d, 0x78, 0x19, 0x42, 0x09, 0xb8, 0xd2, 0x53};

    /* Create Bob's public identity key */
    ec_public_key *bob_identity_key_public;
    result = curve_decode_point(&bob_identity_key_public, bobIdentityPublic, sizeof(bobIdentityPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's private identity key */
    ec_private_key *bob_identity_key_private;
    result = curve_decode_private_point(&bob_identity_key_private, bobIdentityPrivate, sizeof(bobIdentityPrivate), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's identity key pair */
    ratchet_identity_key_pair *bob_identity_key_pair;
    result = ratchet_identity_key_pair_create(&bob_identity_key_pair,
            bob_identity_key_public, bob_identity_key_private);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(bob_identity_key_public);
    SIGNAL_UNREF(bob_identity_key_private);

    /* Create Bob's public ephemeral key */
    ec_public_key *bob_ephemeral_key_public;
    result = curve_decode_point(&bob_ephemeral_key_public, bobPublic, sizeof(bobPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's private ephemeral key */
    ec_private_key *bob_ephemeral_key_private;
    result = curve_decode_private_point(&bob_ephemeral_key_private, bobPrivate, sizeof(bobPrivate), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's ephemeral key pair */
    ec_key_pair *bob_ephemeral_key_pair;
    result = ec_key_pair_create(&bob_ephemeral_key_pair, bob_ephemeral_key_public, bob_ephemeral_key_private);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(bob_ephemeral_key_public);
    SIGNAL_UNREF(bob_ephemeral_key_private);

    /* Create Bob's public signed pre key */
    ec_public_key *bob_signed_pre_key_public;
    result = curve_decode_point(&bob_signed_pre_key_public, bobSignedPreKeyPublic, sizeof(bobSignedPreKeyPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's private signed pre key */
    ec_private_key *bob_signed_pre_key_private;
    result = curve_decode_private_point(&bob_signed_pre_key_private, bobSignedPreKeyPrivate, sizeof(bobSignedPreKeyPrivate), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's signed pre key pair */
    ec_key_pair *bob_signed_pre_key_pair;
    result = ec_key_pair_create(&bob_signed_pre_key_pair, bob_signed_pre_key_public, bob_signed_pre_key_private);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(bob_signed_pre_key_public);
    SIGNAL_UNREF(bob_signed_pre_key_private);

    /* Create Alice's base public key */
    ec_public_key *alice_base_key_public;
    result = curve_decode_point(&alice_base_key_public, aliceBasePublic, sizeof(aliceBasePublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's public identity key */
    ec_public_key *alice_identity_key_public;
    result = curve_decode_point(&alice_identity_key_public, aliceIdentityPublic, sizeof(aliceIdentityPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's Signal Protocol parameters */
    bob_signal_protocol_parameters *bob_parameters;
    result = bob_signal_protocol_parameters_create(&bob_parameters,
            bob_identity_key_pair,
            bob_signed_pre_key_pair,
            0, /* our_one_time_pre_key */
            bob_ephemeral_key_pair,
            alice_identity_key_public,
            alice_base_key_public);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(bob_signed_pre_key_pair);
    SIGNAL_UNREF(bob_ephemeral_key_pair);
    SIGNAL_UNREF(alice_base_key_public);
    /*
     * Not unref'ing the following items that are needed for assertions:
     *   bob_identity_key_pair
     *   alice_identity_key_public
     */

    /* Create the session state */
    session_state *test_session_state;
    result = session_state_create(&test_session_state, global_context);
    ck_assert_int_eq(result, 0);

    result = ratcheting_session_bob_initialize(test_session_state, bob_parameters, global_context);
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(ec_public_key_compare(
            ratchet_identity_key_pair_get_public(bob_identity_key_pair),
            session_state_get_local_identity_key(test_session_state)), 0);

    ck_assert_int_eq(ec_public_key_compare(
            alice_identity_key_public,
            session_state_get_remote_identity_key(test_session_state)), 0);

    ratchet_chain_key *sender_chain_key = session_state_get_sender_chain_key(test_session_state);
    signal_buffer *sender_chain_key_data;
    result = ratchet_chain_key_get_key(sender_chain_key, &sender_chain_key_data);
    ck_assert_int_eq(result, 0);
    int sender_chain_key_size = signal_buffer_len(sender_chain_key_data);
    ck_assert_int_eq(sender_chain_key_size, sizeof(receiverAndSenderChain));
    ck_assert_int_eq(memcmp(signal_buffer_data(sender_chain_key_data), receiverAndSenderChain, sizeof(receiverAndSenderChain)), 0);
    signal_buffer_free(sender_chain_key_data);

    /* Cleanup */
    SIGNAL_UNREF(bob_identity_key_pair);
    SIGNAL_UNREF(alice_identity_key_public);
    SIGNAL_UNREF(bob_parameters);
    SIGNAL_UNREF(test_session_state);
}
END_TEST

START_TEST(test_ratcheting_session_as_alice)
{
    int result = 0;

    uint8_t aliceBasePrivate[] = {
            0x11, 0xae, 0x7c, 0x64, 0xd1, 0xe6, 0x1c, 0xd5,
            0x96, 0xb7, 0x6a, 0x0d, 0xb5, 0x01, 0x26, 0x73,
            0x39, 0x1c, 0xae, 0x66, 0xed, 0xbf, 0xcf, 0x07,
            0x3b, 0x4d, 0xa8, 0x05, 0x16, 0xa4, 0x74, 0x49};

    uint8_t aliceIdentityPrivate[] = {
            0x90, 0x40, 0xf0, 0xd4, 0xe0, 0x9c, 0xf3, 0x8f,
            0x6d, 0xc7, 0xc1, 0x37, 0x79, 0xc9, 0x08, 0xc0,
            0x15, 0xa1, 0xda, 0x4f, 0xa7, 0x87, 0x37, 0xa0,
            0x80, 0xeb, 0x0a, 0x6f, 0x4f, 0x5f, 0x8f, 0x58};

    /* Create Bob's public identity key */
    ec_public_key *bob_identity_key_public;
    result = curve_decode_point(&bob_identity_key_public, bobIdentityPublic, sizeof(bobIdentityPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's public ephemeral key */
    ec_public_key *bob_ephemeral_key_public;
    result = curve_decode_point(&bob_ephemeral_key_public, bobPublic, sizeof(bobPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Bob's public signed pre key */
    ec_public_key *bob_signed_pre_key;
    result = curve_decode_point(&bob_signed_pre_key, bobSignedPreKeyPublic, sizeof(bobSignedPreKeyPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's base public key */
    ec_public_key *alice_base_public_key;
    result = curve_decode_point(&alice_base_public_key, aliceBasePublic, sizeof(aliceBasePublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's base private key */
    ec_private_key *alice_base_private_key;
    result = curve_decode_private_point(&alice_base_private_key, aliceBasePrivate, sizeof(aliceBasePrivate), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's base key pair */
    ec_key_pair *alice_base_key;
    result = ec_key_pair_create(&alice_base_key, alice_base_public_key, alice_base_private_key);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(alice_base_public_key);
    SIGNAL_UNREF(alice_base_private_key);

    /* Create Alice's identity public key */
    ec_public_key *alice_identity_public_key;
    result = curve_decode_point(&alice_identity_public_key, aliceIdentityPublic, sizeof(aliceIdentityPublic), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's identity private key */
    ec_private_key *alice_identity_private_key;
    result = curve_decode_private_point(&alice_identity_private_key, aliceIdentityPrivate, sizeof(aliceIdentityPrivate), global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's identity key pair */
    ratchet_identity_key_pair *alice_identity_key_pair;
    result = ratchet_identity_key_pair_create(&alice_identity_key_pair,
            alice_identity_public_key, alice_identity_private_key);
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(alice_identity_public_key);
    SIGNAL_UNREF(alice_identity_private_key);

    /* Create the session state */
    session_state *test_session_state;
    result = session_state_create(&test_session_state, global_context);
    ck_assert_int_eq(result, 0);

    /* Create Alice's Signal Protocol parameters */
    alice_signal_protocol_parameters *alice_parameters;
    result = alice_signal_protocol_parameters_create(&alice_parameters,
            alice_identity_key_pair, alice_base_key,
            bob_identity_key_public, bob_signed_pre_key, 0,
            bob_ephemeral_key_public);
    ck_assert_int_eq(result, 0);

    result = ratcheting_session_alice_initialize(test_session_state, alice_parameters, global_context);
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(ec_public_key_compare(
            ratchet_identity_key_pair_get_public(alice_identity_key_pair),
            session_state_get_local_identity_key(test_session_state)), 0);

    ck_assert_int_eq(ec_public_key_compare(
            bob_identity_key_public,
            session_state_get_remote_identity_key(test_session_state)), 0);

    ratchet_chain_key *receiver_chain_actual =
            session_state_get_receiver_chain_key(test_session_state, bob_ephemeral_key_public);
    ck_assert_ptr_ne(receiver_chain_actual, 0);

    signal_buffer *receiver_chain_actual_data = 0;
    result = ratchet_chain_key_get_key(receiver_chain_actual, &receiver_chain_actual_data);
    ck_assert_int_eq(result, 0);
    int receiver_chain_actual_data_len = signal_buffer_len(receiver_chain_actual_data);
    ck_assert_int_eq(receiver_chain_actual_data_len, sizeof(receiverAndSenderChain));
    ck_assert_int_eq(memcmp(signal_buffer_data(receiver_chain_actual_data), receiverAndSenderChain, receiver_chain_actual_data_len), 0);
    signal_buffer_free(receiver_chain_actual_data);

    /* Cleanup */
    SIGNAL_UNREF(bob_identity_key_public);
    SIGNAL_UNREF(bob_ephemeral_key_public);
    SIGNAL_UNREF(bob_signed_pre_key);
    SIGNAL_UNREF(alice_base_key);
    SIGNAL_UNREF(alice_identity_key_pair);
    SIGNAL_UNREF(alice_parameters);
    SIGNAL_UNREF(test_session_state);
}
END_TEST

Suite *ratchet_suite(void)
{
    Suite *suite = suite_create("ratchet");

    TCase *tcase_chain_key = tcase_create("chain_key");
    tcase_add_checked_fixture(tcase_chain_key, test_setup, test_teardown);
    tcase_add_test(tcase_chain_key, test_chain_key_derivation_v2);
    tcase_add_test(tcase_chain_key, test_chain_key_derivation_v3);
    suite_add_tcase(suite, tcase_chain_key);

    TCase *tcase_root_key = tcase_create("root_key");
    tcase_add_checked_fixture(tcase_root_key, test_setup, test_teardown);
    tcase_add_test(tcase_root_key, test_root_key_derivation_v2);
    suite_add_tcase(suite, tcase_root_key);

    TCase *tcase_identity_key = tcase_create("identity_key");
    tcase_add_checked_fixture(tcase_identity_key, test_setup, test_teardown);
    tcase_add_test(tcase_identity_key, test_identity_key_serialize);
    suite_add_tcase(suite, tcase_identity_key);

    TCase *tcase_ratcheting_session = tcase_create("ratcheting_session");
    tcase_add_checked_fixture(tcase_ratcheting_session, test_setup, test_teardown);
    tcase_add_test(tcase_ratcheting_session, test_ratcheting_session_as_bob);
    tcase_add_test(tcase_ratcheting_session, test_ratcheting_session_as_alice);
    suite_add_tcase(suite, tcase_ratcheting_session);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = ratchet_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
