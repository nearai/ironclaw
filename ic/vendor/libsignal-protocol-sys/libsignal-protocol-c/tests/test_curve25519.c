#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <check.h>

#include "../src/signal_protocol.h"
#include "../src/signal_protocol_internal.h"
#include "curve.h"
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

START_TEST(test_internal)
{
    ck_assert_int_eq(curve_internal_fast_tests(1), 0);
}
END_TEST

START_TEST(test_curve25519_agreement)
{
    int result;

    uint8_t alicePublic[] = {
            0x05, 0x1b, 0xb7, 0x59, 0x66,
            0xf2, 0xe9, 0x3a, 0x36, 0x91,
            0xdf, 0xff, 0x94, 0x2b, 0xb2,
            0xa4, 0x66, 0xa1, 0xc0, 0x8b,
            0x8d, 0x78, 0xca, 0x3f, 0x4d,
            0x6d, 0xf8, 0xb8, 0xbf, 0xa2,
            0xe4, 0xee, 0x28};

    uint8_t alicePrivate[] = {
            0xc8, 0x06, 0x43, 0x9d, 0xc9,
            0xd2, 0xc4, 0x76, 0xff, 0xed,
            0x8f, 0x25, 0x80, 0xc0, 0x88,
            0x8d, 0x58, 0xab, 0x40, 0x6b,
            0xf7, 0xae, 0x36, 0x98, 0x87,
            0x90, 0x21, 0xb9, 0x6b, 0xb4,
            0xbf, 0x59};

    uint8_t bobPublic[] = {
            0x05, 0x65, 0x36, 0x14, 0x99,
            0x3d, 0x2b, 0x15, 0xee, 0x9e,
            0x5f, 0xd3, 0xd8, 0x6c, 0xe7,
            0x19, 0xef, 0x4e, 0xc1, 0xda,
            0xae, 0x18, 0x86, 0xa8, 0x7b,
            0x3f, 0x5f, 0xa9, 0x56, 0x5a,
            0x27, 0xa2, 0x2f};

    uint8_t bobPrivate[] = {
            0xb0, 0x3b, 0x34, 0xc3, 0x3a,
            0x1c, 0x44, 0xf2, 0x25, 0xb6,
            0x62, 0xd2, 0xbf, 0x48, 0x59,
            0xb8, 0x13, 0x54, 0x11, 0xfa,
            0x7b, 0x03, 0x86, 0xd4, 0x5f,
            0xb7, 0x5d, 0xc5, 0xb9, 0x1b,
            0x44, 0x66};

    uint8_t shared[] = {
            0x32, 0x5f, 0x23, 0x93, 0x28,
            0x94, 0x1c, 0xed, 0x6e, 0x67,
            0x3b, 0x86, 0xba, 0x41, 0x01,
            0x74, 0x48, 0xe9, 0x9b, 0x64,
            0x9a, 0x9c, 0x38, 0x06, 0xc1,
            0xdd, 0x7c, 0xa4, 0xc4, 0x77,
            0xe6, 0x29};

    ec_public_key *alice_public_key = 0;
    ec_private_key *alice_private_key = 0;
    ec_public_key *bob_public_key = 0;
    ec_private_key *bob_private_key = 0;
    uint8_t *shared_one = 0;
    uint8_t *shared_two = 0;

    /* Initialize Alice's public key */
    result = curve_decode_point(&alice_public_key, alicePublic, sizeof(alicePublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_public_key, 0);

    /* Initialize Alice's private key */
    result = curve_decode_private_point(&alice_private_key, alicePrivate, sizeof(alicePrivate), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_private_key, 0);

    /* Initialize Bob's public key */
    result = curve_decode_point(&bob_public_key, bobPublic, sizeof(bobPublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(bob_public_key, 0);

    /* Initialize Bob's private key */
    result = curve_decode_private_point(&bob_private_key, bobPrivate, sizeof(bobPrivate), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(bob_private_key, 0);

    /* Calculate key agreement one */
    result = curve_calculate_agreement(&shared_one, alice_public_key, bob_private_key);
    ck_assert_int_eq(result, 32);
    ck_assert_ptr_ne(shared_one, 0);

    /* Calculate key agreement two */
    result = curve_calculate_agreement(&shared_two, bob_public_key, alice_private_key);
    ck_assert_int_eq(result, 32);
    ck_assert_ptr_ne(shared_two, 0);

    /* Assert that key agreements are correct */
    ck_assert_int_eq(memcmp(shared_one, shared, 32), 0);
    ck_assert_int_eq(memcmp(shared_two, shared, 32), 0);

    /* Cleanup */
    if(shared_one) { free(shared_one); }
    if(shared_two) { free(shared_two); }
    SIGNAL_UNREF(alice_public_key);
    SIGNAL_UNREF(alice_private_key);
    SIGNAL_UNREF(bob_public_key);
    SIGNAL_UNREF(bob_private_key);
}
END_TEST

START_TEST(test_curve25519_generate_public)
{
    int result;

    uint8_t alicePublic[] = {
            0x05, 0x1b, 0xb7, 0x59, 0x66,
            0xf2, 0xe9, 0x3a, 0x36, 0x91,
            0xdf, 0xff, 0x94, 0x2b, 0xb2,
            0xa4, 0x66, 0xa1, 0xc0, 0x8b,
            0x8d, 0x78, 0xca, 0x3f, 0x4d,
            0x6d, 0xf8, 0xb8, 0xbf, 0xa2,
            0xe4, 0xee, 0x28};

    uint8_t alicePrivate[] = {
            0xc8, 0x06, 0x43, 0x9d, 0xc9,
            0xd2, 0xc4, 0x76, 0xff, 0xed,
            0x8f, 0x25, 0x80, 0xc0, 0x88,
            0x8d, 0x58, 0xab, 0x40, 0x6b,
            0xf7, 0xae, 0x36, 0x98, 0x87,
            0x90, 0x21, 0xb9, 0x6b, 0xb4,
            0xbf, 0x59};

    ec_private_key *alice_private_key = 0;
    ec_public_key *alice_expected_public_key = 0;
    ec_public_key *alice_public_key = 0;

    /* Initialize Alice's private key */
    result = curve_decode_private_point(&alice_private_key, alicePrivate, sizeof(alicePrivate), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_private_key, 0);

    /* Initialize Alice's expected public key */
    result = curve_decode_point(&alice_expected_public_key, alicePublic, sizeof(alicePublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_expected_public_key, 0);

    /* Generate Alice's actual public key */
    result = curve_generate_public_key(&alice_public_key, alice_private_key);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_public_key, 0);

    /* Assert that expected and actual public keys match */
    ck_assert_int_eq(ec_public_key_compare(alice_expected_public_key, alice_public_key), 0);

    /* Cleanup */
    SIGNAL_UNREF(alice_public_key);
    SIGNAL_UNREF(alice_expected_public_key);
    SIGNAL_UNREF(alice_private_key);
}
END_TEST

START_TEST(test_curve25519_random_agreements)
{
    int result;
    int i;

    ec_key_pair *alice_key_pair = 0;
    ec_public_key *alice_public_key = 0;
    ec_private_key *alice_private_key = 0;
    ec_key_pair *bob_key_pair = 0;
    ec_public_key *bob_public_key = 0;
    ec_private_key *bob_private_key = 0;
    uint8_t *shared_alice = 0;
    uint8_t *shared_bob = 0;

    signal_context *context;
    signal_context_create(&context, 0);
    setup_test_crypto_provider(context);

    for(i = 0; i < 50; i++) {
        /* Generate Alice's key pair */
        result = curve_generate_key_pair(context, &alice_key_pair);
        ck_assert_int_eq(result, 0);
        alice_public_key = ec_key_pair_get_public(alice_key_pair);
        alice_private_key = ec_key_pair_get_private(alice_key_pair);
        ck_assert_ptr_ne(alice_public_key, 0);
        ck_assert_ptr_ne(alice_private_key, 0);

        /* Generate Bob's key pair */
        result = curve_generate_key_pair(context, &bob_key_pair);
        ck_assert_int_eq(result, 0);
        bob_public_key = ec_key_pair_get_public(bob_key_pair);
        bob_private_key = ec_key_pair_get_private(bob_key_pair);
        ck_assert_ptr_ne(bob_public_key, 0);
        ck_assert_ptr_ne(bob_private_key, 0);

        /* Calculate Alice's key agreement */
        result = curve_calculate_agreement(&shared_alice, bob_public_key, alice_private_key);
        ck_assert_int_eq(result, 32);
        ck_assert_ptr_ne(shared_alice, 0);

        /* Calculate Bob's key agreement */
        result = curve_calculate_agreement(&shared_bob, alice_public_key, bob_private_key);
        ck_assert_int_eq(result, 32);
        ck_assert_ptr_ne(shared_bob, 0);

        /* Assert that key agreements match */
        ck_assert_int_eq(memcmp(shared_alice, shared_bob, 32), 0);

        /* Cleanup */
        if(shared_alice) { free(shared_alice); }
        if(shared_bob) { free(shared_bob); }
        SIGNAL_UNREF(alice_key_pair);
        SIGNAL_UNREF(bob_key_pair);
        alice_key_pair = 0;
        bob_key_pair = 0;
        alice_public_key = 0;
        alice_private_key = 0;
        bob_public_key = 0;
        bob_private_key = 0;
        shared_alice = 0;
        shared_bob = 0;
    }

    signal_context_destroy(context);
}
END_TEST

START_TEST(test_curve25519_signature)
{
    int result;

    uint8_t aliceIdentityPrivate[] = {
            0xc0, 0x97, 0x24, 0x84, 0x12, 0xe5, 0x8b, 0xf0,
            0x5d, 0xf4, 0x87, 0x96, 0x82, 0x05, 0x13, 0x27,
            0x94, 0x17, 0x8e, 0x36, 0x76, 0x37, 0xf5, 0x81,
            0x8f, 0x81, 0xe0, 0xe6, 0xce, 0x73, 0xe8, 0x65};

    uint8_t aliceIdentityPublic[] = {
            0x05, 0xab, 0x7e, 0x71, 0x7d, 0x4a, 0x16, 0x3b,
            0x7d, 0x9a, 0x1d, 0x80, 0x71, 0xdf, 0xe9, 0xdc,
            0xf8, 0xcd, 0xcd, 0x1c, 0xea, 0x33, 0x39, 0xb6,
            0x35, 0x6b, 0xe8, 0x4d, 0x88, 0x7e, 0x32, 0x2c,
            0x64};

    uint8_t aliceEphemeralPublic[] = {
            0x05, 0xed, 0xce, 0x9d, 0x9c, 0x41, 0x5c, 0xa7,
            0x8c, 0xb7, 0x25, 0x2e, 0x72, 0xc2, 0xc4, 0xa5,
            0x54, 0xd3, 0xeb, 0x29, 0x48, 0x5a, 0x0e, 0x1d,
            0x50, 0x31, 0x18, 0xd1, 0xa8, 0x2d, 0x99, 0xfb,
            0x4a};

    uint8_t aliceSignature[] = {
            0x5d, 0xe8, 0x8c, 0xa9, 0xa8, 0x9b, 0x4a, 0x11,
            0x5d, 0xa7, 0x91, 0x09, 0xc6, 0x7c, 0x9c, 0x74,
            0x64, 0xa3, 0xe4, 0x18, 0x02, 0x74, 0xf1, 0xcb,
            0x8c, 0x63, 0xc2, 0x98, 0x4e, 0x28, 0x6d, 0xfb,
            0xed, 0xe8, 0x2d, 0xeb, 0x9d, 0xcd, 0x9f, 0xae,
            0x0b, 0xfb, 0xb8, 0x21, 0x56, 0x9b, 0x3d, 0x90,
            0x01, 0xbd, 0x81, 0x30, 0xcd, 0x11, 0xd4, 0x86,
            0xce, 0xf0, 0x47, 0xbd, 0x60, 0xb8, 0x6e, 0x88};

    ec_private_key *alice_private_key = 0;
    ec_public_key *alice_public_key = 0;
    ec_public_key *alice_ephemeral = 0;

    /* Initialize Alice's private key */
    result = curve_decode_private_point(&alice_private_key, aliceIdentityPrivate, sizeof(aliceIdentityPrivate), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_private_key, 0);

    /* Initialize Alice's public key */
    result = curve_decode_point(&alice_public_key, aliceIdentityPublic, sizeof(aliceIdentityPublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_public_key, 0);

    /* Initialize Alice's ephemeral key */
    result = curve_decode_point(&alice_ephemeral, aliceEphemeralPublic, sizeof(aliceEphemeralPublic), global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(alice_ephemeral, 0);

    result = curve_verify_signature(alice_public_key,
            aliceEphemeralPublic, sizeof(aliceEphemeralPublic),
            aliceSignature, sizeof(aliceSignature));
    ck_assert_msg(result == 1, "signature verification failed");

    uint8_t modifiedSignature[sizeof(aliceSignature)];

    int i;
    for(i = 0; i < sizeof(aliceSignature); i++) {
        memcpy(modifiedSignature, aliceSignature, sizeof(aliceSignature));
        modifiedSignature[i] ^= 0x01;

        result = curve_verify_signature(alice_public_key,
                aliceEphemeralPublic, sizeof(aliceEphemeralPublic),
                modifiedSignature, sizeof(modifiedSignature));
        ck_assert_msg(result != 1, "signature verification succeeded");
    }

    /* Cleanup */
    SIGNAL_UNREF(alice_private_key);
    SIGNAL_UNREF(alice_public_key);
    SIGNAL_UNREF(alice_ephemeral);
}
END_TEST

START_TEST(test_curve25519_large_signatures)
{
    int result;

    ec_key_pair *keys = 0;
    result = curve_generate_key_pair(global_context, &keys);
    ck_assert_int_eq(result, 0);

    uint8_t message[1048576];
    memset(message, 0, sizeof(message));

    signal_buffer *signature = 0;

    result = curve_calculate_signature(global_context, &signature,
            ec_key_pair_get_private(keys), message, sizeof(message));
    ck_assert_int_eq(result, 0);

    uint8_t *data = signal_buffer_data(signature);
    size_t len = signal_buffer_len(signature);

    result = curve_verify_signature(ec_key_pair_get_public(keys),
            message, sizeof(message), data, len);
    ck_assert_int_eq(result, 1);

    data[0] ^= 0x01;

    result = curve_verify_signature(ec_key_pair_get_public(keys),
            message, sizeof(message), data, len);
    ck_assert_int_eq(result, 0);

    /* Cleanup */
    SIGNAL_UNREF(keys);
    if(signature) {
        signal_buffer_free(signature);
    }
}
END_TEST

START_TEST(test_unique_signatures)
{
    int result;
    size_t i;
    size_t r;
    ec_key_pair *key_pair = 0;
    uint8_t *message = 0;
    signal_buffer *signature = 0;
    signal_buffer *vrf_output = 0;

    result = curve_generate_key_pair(global_context, &key_pair);
    ck_assert_int_eq(result, 0);

    message = malloc(256);
    ck_assert_ptr_ne(message, 0);

    for(i = 1; i <= 256; i++) {
        result = signal_crypto_random(global_context, message, i);
        ck_assert_int_eq(result, 0);

        result = curve_calculate_vrf_signature(global_context, &signature,
                ec_key_pair_get_private(key_pair), message, i);
        ck_assert_int_eq(result, 0);

        result = curve_verify_vrf_signature(global_context, &vrf_output,
                ec_key_pair_get_public(key_pair), message, i,
                signal_buffer_data(signature), signal_buffer_len(signature));
        ck_assert_int_eq(result, 0);

        result = curve_verify_signature(
                ec_key_pair_get_public(key_pair), message, i,
                signal_buffer_data(signature), signal_buffer_len(signature));
        ck_assert_int_ne(result, 0);

        signal_buffer_free(vrf_output);

        result = signal_crypto_random(global_context, (uint8_t *)&r, sizeof(size_t));
        ck_assert_int_eq(result, 0);

        message[r % i] ^= 0x01;

        result = curve_verify_vrf_signature(global_context, &vrf_output,
                ec_key_pair_get_public(key_pair), message, i,
                signal_buffer_data(signature), signal_buffer_len(signature));
        ck_assert_int_eq(result, SG_ERR_VRF_SIG_VERIF_FAILED);

        signal_buffer_free(signature);
    }

    /* Cleanup */
    SIGNAL_UNREF(key_pair);
    if(message) {
        free(message);
    }
}
END_TEST

START_TEST(test_unique_signature_vector)
{
    uint8_t publicKey[] = {
            0x05,
            0x21, 0xf7, 0x34, 0x5f, 0x56, 0xd9, 0x60, 0x2f,
            0x15, 0x23, 0x29, 0x8f, 0x4f, 0x6f, 0xce, 0xcb,
            0x14, 0xdd, 0xe2, 0xd5, 0xb9, 0xa9, 0xb4, 0x8b,
            0xca, 0x82, 0x42, 0x68, 0x14, 0x92, 0xb9, 0x20};
    uint8_t privateKey[] = {
            0x38, 0x61, 0x1d, 0x25, 0x3b, 0xea, 0x85, 0xa2,
            0x03, 0x80, 0x53, 0x43, 0xb7, 0x4a, 0x93, 0x6d,
            0x3b, 0x13, 0xb9, 0xe3, 0x12, 0x14, 0x53, 0xe9,
            0x74, 0x0b, 0x6b, 0x82, 0x7e, 0x33, 0x7e, 0x5d};
    uint8_t message[] = {
            0x54, 0x68, 0x69, 0x73, 0x20, 0x69, 0x73, 0x20,
            0x75, 0x6e, 0x69, 0x71, 0x75, 0x65, 0x2e};
    uint8_t vrf[] = {
            0x45, 0xDC, 0x7B, 0x81, 0x6B, 0x01, 0xB3, 0x6C, 
            0xFA, 0x16, 0x45, 0xDC, 0xAE, 0x8A, 0xC9, 0xBC, 
            0x8E, 0x52, 0x3C, 0xD8, 0x6D, 0x00, 0x7D, 0x19, 
            0x95, 0x3F, 0x03, 0xE7, 0xD5, 0x45, 0x54, 0xA0
            };

    int result;
    ec_public_key *public_key = 0;
    ec_private_key *private_key = 0;
    signal_buffer *signature = 0;
    signal_buffer *vrf_output = 0;

    result = curve_decode_point(&public_key, publicKey, sizeof(publicKey), global_context);
    ck_assert_int_eq(result, 0);

    result = curve_decode_private_point(&private_key, privateKey, sizeof(privateKey), global_context);
    ck_assert_int_eq(result, 0);

    result = curve_calculate_vrf_signature(global_context, &signature,
            private_key, message, sizeof(message));
    ck_assert_int_eq(result, 0);

    result = curve_verify_vrf_signature(global_context, &vrf_output,
            public_key, message, sizeof(message),
            signal_buffer_data(signature), signal_buffer_len(signature));
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(signal_buffer_len(vrf_output), sizeof(vrf));
    ck_assert_int_eq(memcmp(signal_buffer_data(vrf_output), vrf, sizeof(vrf)), 0);

    /* Cleanup */
    signal_buffer_free(signature);
    signal_buffer_free(vrf_output);
    SIGNAL_UNREF(public_key);
    SIGNAL_UNREF(private_key);
}
END_TEST

Suite *curve25519_suite(void)
{
    Suite *suite = suite_create("curve25519");
    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_internal);
    tcase_add_test(tcase, test_curve25519_agreement);
    tcase_add_test(tcase, test_curve25519_generate_public);
    tcase_add_test(tcase, test_curve25519_random_agreements);
    tcase_add_test(tcase, test_curve25519_signature);
    tcase_add_test(tcase, test_curve25519_large_signatures);
    tcase_add_test(tcase, test_unique_signatures);
    tcase_add_test(tcase, test_unique_signature_vector);
    suite_add_tcase(suite, tcase);
    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = curve25519_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
