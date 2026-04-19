#include <check.h>

#include "../src/signal_protocol.h"
#include "device_consistency.h"
#include "curve.h"
#include "test_common.h"

signal_context *global_context;

static char *generate_code(device_consistency_commitment *commitment,
        device_consistency_message *msg1,
        device_consistency_message *msg2,
        device_consistency_message *msg3);

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

START_TEST(test_device_consistency)
{
    int result = 0;
    int i;

    /* Create three device key pairs */
    ec_key_pair *device_one = 0;
    result = curve_generate_key_pair(global_context, &device_one);
    ck_assert_int_eq(result, 0);

    ec_key_pair *device_two = 0;
    result = curve_generate_key_pair(global_context, &device_two);
    ck_assert_int_eq(result, 0);

    ec_key_pair *device_three = 0;
    result = curve_generate_key_pair(global_context, &device_three);
    ck_assert_int_eq(result, 0);

    ec_public_key *key_array[] = {
            ec_key_pair_get_public(device_one),
            ec_key_pair_get_public(device_two),
            ec_key_pair_get_public(device_three)
    };

    /* Create device one commitment */
    ec_public_key_list *key_list = ec_public_key_list_alloc();
    ck_assert_ptr_ne(key_list, 0);
    shuffle_ec_public_keys(key_array, 3);
    for(i = 0; i < 3; i++) {
        ec_public_key_list_push_back(key_list, key_array[i]);
    }

    device_consistency_commitment *device_one_commitment = 0;
    result = device_consistency_commitment_create(&device_one_commitment, 1, key_list, global_context);
    ck_assert_int_eq(result, 0);
    ec_public_key_list_free(key_list);

    /* Create device two commitment */
    key_list = ec_public_key_list_alloc();
    ck_assert_ptr_ne(key_list, 0);
    shuffle_ec_public_keys(key_array, 3);
    for(i = 0; i < 3; i++) {
        ec_public_key_list_push_back(key_list, key_array[i]);
    }

    device_consistency_commitment *device_two_commitment = 0;
    result = device_consistency_commitment_create(&device_two_commitment, 1, key_list, global_context);
    ck_assert_int_eq(result, 0);
    ec_public_key_list_free(key_list);

    /* Create device three commitment */
    key_list = ec_public_key_list_alloc();
    ck_assert_ptr_ne(key_list, 0);
    shuffle_ec_public_keys(key_array, 3);
    for(i = 0; i < 3; i++) {
        ec_public_key_list_push_back(key_list, key_array[i]);
    }

    device_consistency_commitment *device_three_commitment = 0;
    result = device_consistency_commitment_create(&device_three_commitment, 1, key_list, global_context);
    ck_assert_int_eq(result, 0);
    ec_public_key_list_free(key_list);

    /* Check that all three commitments are equal */
    ck_assert_int_eq(signal_buffer_compare(
            device_consistency_commitment_get_serialized(device_one_commitment),
            device_consistency_commitment_get_serialized(device_two_commitment)), 0);
    ck_assert_int_eq(signal_buffer_compare(
            device_consistency_commitment_get_serialized(device_two_commitment),
            device_consistency_commitment_get_serialized(device_three_commitment)), 0);

    /* Create device consistency messages */
    device_consistency_message *device_one_message = 0;
    result = device_consistency_message_create_from_pair(&device_one_message,
            device_one_commitment, device_one, global_context);
    ck_assert_int_eq(result, 0);

    device_consistency_message *device_two_message = 0;
    result = device_consistency_message_create_from_pair(&device_two_message,
            device_one_commitment, device_two, global_context);
    ck_assert_int_eq(result, 0);

    device_consistency_message *device_three_message = 0;
    result = device_consistency_message_create_from_pair(&device_three_message,
            device_one_commitment, device_three, global_context);
    ck_assert_int_eq(result, 0);

    /* Create received device consistency messages */
    signal_buffer *device_one_message_serialized =
            device_consistency_message_get_serialized(device_one_message);
    device_consistency_message *received_device_one_message = 0;
    result = device_consistency_message_create_from_serialized(&received_device_one_message,
            device_one_commitment,
            signal_buffer_data(device_one_message_serialized),
            signal_buffer_len(device_one_message_serialized),
            ec_key_pair_get_public(device_one), global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *device_two_message_serialized =
            device_consistency_message_get_serialized(device_two_message);
    device_consistency_message *received_device_two_message = 0;
    result = device_consistency_message_create_from_serialized(&received_device_two_message,
            device_one_commitment,
            signal_buffer_data(device_two_message_serialized),
            signal_buffer_len(device_two_message_serialized),
            ec_key_pair_get_public(device_two), global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *device_three_message_serialized =
            device_consistency_message_get_serialized(device_three_message);
    device_consistency_message *received_device_three_message = 0;
    result = device_consistency_message_create_from_serialized(&received_device_three_message,
            device_one_commitment,
            signal_buffer_data(device_three_message_serialized),
            signal_buffer_len(device_three_message_serialized),
            ec_key_pair_get_public(device_three), global_context);
    ck_assert_int_eq(result, 0);

    /* Check that all sent-and-received pairs have the same VRF output */
    ck_assert_int_eq(signal_buffer_compare(
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(device_one_message)),
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(received_device_one_message))),
            0);
    ck_assert_int_eq(signal_buffer_compare(
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(device_two_message)),
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(received_device_two_message))),
            0);
    ck_assert_int_eq(signal_buffer_compare(
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(device_three_message)),
            device_consistency_signature_get_vrf_output(
                    device_consistency_message_get_signature(received_device_three_message))),
            0);

    /* Generate consistency codes */
    char *code_one = generate_code(device_one_commitment,
            device_one_message, received_device_two_message, received_device_three_message);
    char *code_two = generate_code(device_two_commitment,
            device_two_message, received_device_three_message, received_device_one_message);
    char *code_three = generate_code(device_three_commitment,
            device_three_message, received_device_two_message, received_device_one_message);

    /* Check that all the consistency codes match */
    ck_assert_str_eq(code_one, code_two);
    ck_assert_str_eq(code_two, code_three);

    /* Cleanup */
    SIGNAL_UNREF(device_one);
    SIGNAL_UNREF(device_two);
    SIGNAL_UNREF(device_three);
    SIGNAL_UNREF(device_one_commitment);
    SIGNAL_UNREF(device_two_commitment);
    SIGNAL_UNREF(device_three_commitment);
    SIGNAL_UNREF(device_one_message);
    SIGNAL_UNREF(device_two_message);
    SIGNAL_UNREF(device_three_message);
    SIGNAL_UNREF(received_device_one_message);
    SIGNAL_UNREF(received_device_two_message);
    SIGNAL_UNREF(received_device_three_message);
    free(code_one);
    free(code_two);
    free(code_three);
}
END_TEST

char *generate_code(device_consistency_commitment *commitment,
        device_consistency_message *msg1,
        device_consistency_message *msg2,
        device_consistency_message *msg3)
{
    int result = 0;
    char *code_string = 0;

    /* Build the list of signatures */
    device_consistency_signature_list *signatures = device_consistency_signature_list_alloc();
    ck_assert_ptr_ne(signatures, 0);

    device_consistency_signature_list_push_back(signatures,
            device_consistency_message_get_signature(msg1));
    device_consistency_signature_list_push_back(signatures,
            device_consistency_message_get_signature(msg2));
    device_consistency_signature_list_push_back(signatures,
            device_consistency_message_get_signature(msg3));

    result = device_consistency_code_generate_for(commitment, signatures, &code_string, global_context);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(code_string, 0);

    device_consistency_signature_list_free(signatures);

    return code_string;
}

Suite *device_consistency_suite(void)
{
    Suite *suite = suite_create("device_consistency");

    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_device_consistency);
    suite_add_tcase(suite, tcase);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = device_consistency_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
