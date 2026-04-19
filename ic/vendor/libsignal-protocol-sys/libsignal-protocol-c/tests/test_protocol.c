#include <stdio.h>
#include <stdlib.h>
#include <check.h>

#include "../src/signal_protocol.h"
#include "curve.h"
#include "protocol.h"
#include "ratchet.h"
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

void compare_signal_messages(signal_message *message1, signal_message *message2)
{
    ec_public_key *sender_ratchet_key1 = signal_message_get_sender_ratchet_key(message1);
    ec_public_key *sender_ratchet_key2 = signal_message_get_sender_ratchet_key(message2);
    ck_assert_int_eq(ec_public_key_compare(sender_ratchet_key1, sender_ratchet_key2), 0);

    int version1 = signal_message_get_message_version(message1);
    int version2 = signal_message_get_message_version(message2);
    ck_assert_int_eq(version1, version2);

    int counter1 = signal_message_get_counter(message1);
    int counter2 = signal_message_get_counter(message2);
    ck_assert_int_eq(counter1, counter2);

    signal_buffer *body1 = signal_message_get_body(message1);
    signal_buffer *body2 = signal_message_get_body(message2);
    ck_assert_int_eq(signal_buffer_compare(body1, body2), 0);
}

START_TEST(test_serialize_signal_message)
{
    int result = 0;

    static const char ciphertext[] = "WhisperCipherText";
    ec_public_key *sender_ratchet_key = create_test_ec_public_key(global_context);
    ec_public_key *sender_identity_key = create_test_ec_public_key(global_context);
    ec_public_key *receiver_identity_key = create_test_ec_public_key(global_context);
    uint8_t mac_key[RATCHET_MAC_KEY_LENGTH];
    memset(mac_key, 1, sizeof(mac_key));

    signal_message *message = 0;
    signal_message *result_message = 0;

    result = signal_message_create(&message, 3,
            mac_key, sizeof(mac_key),
            sender_ratchet_key,
            2, /* counter */
            1, /* previous counter */
            (uint8_t *)ciphertext, sizeof(ciphertext) - 1,
            sender_identity_key, receiver_identity_key,
            global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *serialized = ciphertext_message_get_serialized((ciphertext_message *)message);
    ck_assert_ptr_ne(serialized, 0);

    result = signal_message_deserialize(&result_message,
            signal_buffer_data(serialized),
            signal_buffer_len(serialized),
            global_context);
    ck_assert_int_eq(result, 0);

    compare_signal_messages(message, result_message);

    /* Exercise the MAC verification code */
    result = signal_message_verify_mac(result_message,
            sender_identity_key, receiver_identity_key,
            mac_key, sizeof(mac_key), global_context);
    ck_assert_int_eq(result, 1);

    /* Cleanup */
    SIGNAL_UNREF(message);
    SIGNAL_UNREF(result_message);
    SIGNAL_UNREF(sender_ratchet_key);
    SIGNAL_UNREF(sender_identity_key);
    SIGNAL_UNREF(receiver_identity_key);
}
END_TEST

START_TEST(test_serialize_pre_key_signal_message)
{
    int result = 0;

    static const char ciphertext[] = "WhisperCipherText";
    ec_public_key *sender_ratchet_key = create_test_ec_public_key(global_context);
    ec_public_key *sender_identity_key = create_test_ec_public_key(global_context);
    ec_public_key *receiver_identity_key = create_test_ec_public_key(global_context);
    ec_public_key *base_key = create_test_ec_public_key(global_context);
    ec_public_key *identity_key = create_test_ec_public_key(global_context);
    uint8_t mac_key[RATCHET_MAC_KEY_LENGTH];
    memset(mac_key, 1, sizeof(mac_key));

    signal_message *message = 0;
    pre_key_signal_message *pre_key_message = 0;
    pre_key_signal_message *result_pre_key_message = 0;

    result = signal_message_create(&message, 3,
            mac_key, sizeof(mac_key),
            sender_ratchet_key,
            2, /* counter */
            1, /* previous counter */
            (uint8_t *)ciphertext, sizeof(ciphertext) - 1,
            sender_identity_key, receiver_identity_key,
            global_context);
    ck_assert_int_eq(result, 0);

    uint32_t pre_key_id = 56;
    result = pre_key_signal_message_create(&pre_key_message,
            3,  /* message version */
            42, /* registration ID */
            &pre_key_id, /* pre key ID */
            72, /* signed pre key ID */
            base_key, identity_key,
            message,
            global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *serialized = ciphertext_message_get_serialized((ciphertext_message *)pre_key_message);
    ck_assert_ptr_ne(serialized, 0);

    result = pre_key_signal_message_deserialize(&result_pre_key_message,
            signal_buffer_data(serialized),
            signal_buffer_len(serialized),
            global_context);
    ck_assert_int_eq(result, 0);

    int version1 = pre_key_signal_message_get_message_version(pre_key_message);
    int version2 = pre_key_signal_message_get_message_version(result_pre_key_message);
    ck_assert_int_eq(version1, version2);

    ec_public_key *identity_key1 = pre_key_signal_message_get_identity_key(pre_key_message);
    ec_public_key *identity_key2 = pre_key_signal_message_get_identity_key(result_pre_key_message);
    ck_assert_int_eq(ec_public_key_compare(identity_key1, identity_key2), 0);

    int registration_id1 = pre_key_signal_message_get_registration_id(pre_key_message);
    int registration_id2 = pre_key_signal_message_get_registration_id(result_pre_key_message);
    ck_assert_int_eq(registration_id1, registration_id2);

    int has_pre_key_id1 = pre_key_signal_message_has_pre_key_id(pre_key_message);
    int has_pre_key_id2 = pre_key_signal_message_has_pre_key_id(result_pre_key_message);
    ck_assert_int_eq(has_pre_key_id1, has_pre_key_id2);

    if(has_pre_key_id1) {
        int pre_key_id1 = pre_key_signal_message_get_pre_key_id(pre_key_message);
        int pre_key_id2 = pre_key_signal_message_get_pre_key_id(result_pre_key_message);
        ck_assert_int_eq(pre_key_id1, pre_key_id2);
    }

    int signed_pre_key_id1 = pre_key_signal_message_get_signed_pre_key_id(pre_key_message);
    int signed_pre_key_id2 = pre_key_signal_message_get_signed_pre_key_id(result_pre_key_message);
    ck_assert_int_eq(signed_pre_key_id1, signed_pre_key_id2);

    ec_public_key *base_key1 = pre_key_signal_message_get_base_key(pre_key_message);
    ec_public_key *base_key2 = pre_key_signal_message_get_base_key(result_pre_key_message);
    ck_assert_int_eq(ec_public_key_compare(base_key1, base_key2), 0);

    signal_message *message1 = pre_key_signal_message_get_signal_message(pre_key_message);
    signal_message *message2 = pre_key_signal_message_get_signal_message(result_pre_key_message);
    compare_signal_messages(message1, message2);

    /* Cleanup */
    SIGNAL_UNREF(message);
    SIGNAL_UNREF(result_pre_key_message);
    SIGNAL_UNREF(pre_key_message);
    SIGNAL_UNREF(sender_ratchet_key);
    SIGNAL_UNREF(sender_identity_key);
    SIGNAL_UNREF(receiver_identity_key);
    SIGNAL_UNREF(base_key);
    SIGNAL_UNREF(identity_key);
}
END_TEST

START_TEST(test_serialize_sender_key_message)
{
    int result = 0;
    sender_key_message *message = 0;
    sender_key_message *result_message = 0;
    static const char ciphertext[] = "WhisperCipherText";
    ec_key_pair *signature_key_pair = 0;

    result = curve_generate_key_pair(global_context, &signature_key_pair);
    ck_assert_int_eq(result, 0);

    result = sender_key_message_create(&message,
            10, /* key_id */
            1,  /* iteration */
            (uint8_t *)ciphertext, sizeof(ciphertext) - 1,
            ec_key_pair_get_private(signature_key_pair),
            global_context);
    ck_assert_int_eq(result, 0);

    result = sender_key_message_verify_signature(message, ec_key_pair_get_public(signature_key_pair));
    ck_assert_int_eq(result, 0);

    signal_buffer *serialized = ciphertext_message_get_serialized((ciphertext_message *)message);
    ck_assert_ptr_ne(serialized, 0);

    result = sender_key_message_deserialize(&result_message,
            signal_buffer_data(serialized),
            signal_buffer_len(serialized),
            global_context);
    ck_assert_int_eq(result, 0);

    result = sender_key_message_verify_signature(result_message, ec_key_pair_get_public(signature_key_pair));
    ck_assert_int_eq(result, 0);

    int key_id1 = sender_key_message_get_key_id(message);
    int key_id2 = sender_key_message_get_key_id(result_message);
    ck_assert_int_eq(key_id1, key_id2);

    int iteration1 = sender_key_message_get_iteration(message);
    int iteration2 = sender_key_message_get_iteration(result_message);
    ck_assert_int_eq(iteration1, iteration2);

    signal_buffer *ciphertext1 = sender_key_message_get_ciphertext(message);
    signal_buffer *ciphertext2 = sender_key_message_get_ciphertext(result_message);
    ck_assert_int_eq(signal_buffer_compare(ciphertext1, ciphertext2), 0);

    /* Cleanup */
    SIGNAL_UNREF(message);
    SIGNAL_UNREF(result_message);
    SIGNAL_UNREF(signature_key_pair);
}
END_TEST

START_TEST(test_serialize_sender_key_distribution_message)
{
    int result = 0;
    sender_key_distribution_message *message = 0;
    sender_key_distribution_message *result_message = 0;
    static const char chain_key[] = "WhisperChainKey";
    ec_public_key *signature_key = create_test_ec_public_key(global_context);

    result = sender_key_distribution_message_create(&message,
            10, /* id */
            1,  /* iteration */
            (uint8_t *)chain_key, sizeof(chain_key) - 1,
            signature_key,
            global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *serialized = ciphertext_message_get_serialized((ciphertext_message *)message);
    ck_assert_ptr_ne(serialized, 0);

    result = sender_key_distribution_message_deserialize(&result_message,
            signal_buffer_data(serialized),
            signal_buffer_len(serialized),
            global_context);
    ck_assert_int_eq(result, 0);

    int id1 = sender_key_distribution_message_get_id(message);
    int id2 = sender_key_distribution_message_get_id(result_message);
    ck_assert_int_eq(id1, id2);

    int iteration1 = sender_key_distribution_message_get_iteration(message);
    int iteration2 = sender_key_distribution_message_get_iteration(result_message);
    ck_assert_int_eq(iteration1, iteration2);

    signal_buffer *chain_key1 = sender_key_distribution_message_get_chain_key(message);
    signal_buffer *chain_key2 = sender_key_distribution_message_get_chain_key(result_message);
    ck_assert_int_eq(signal_buffer_compare(chain_key1, chain_key2), 0);

    ec_public_key *signature_key1 = sender_key_distribution_message_get_signature_key(message);
    ec_public_key *signature_key2 = sender_key_distribution_message_get_signature_key(result_message);
    ck_assert_int_eq(ec_public_key_compare(signature_key1, signature_key2), 0);

    /* Cleanup */
    SIGNAL_UNREF(message);
    SIGNAL_UNREF(result_message);
    SIGNAL_UNREF(signature_key);
}
END_TEST

Suite *protocol_suite(void)
{
    Suite *suite = suite_create("protocol");

    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_serialize_signal_message);
    tcase_add_test(tcase, test_serialize_pre_key_signal_message);
    tcase_add_test(tcase, test_serialize_sender_key_message);
    tcase_add_test(tcase, test_serialize_sender_key_distribution_message);
    suite_add_tcase(suite, tcase);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = protocol_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
