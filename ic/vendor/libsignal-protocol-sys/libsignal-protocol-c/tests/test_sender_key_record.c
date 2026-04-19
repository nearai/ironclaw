#include <stdio.h>
#include <stdlib.h>
#include <check.h>

#include "../src/signal_protocol.h"
#include "key_helper.h"
#include "curve.h"
#include "sender_key.h"
#include "sender_key_state.h"
#include "sender_key_record.h"
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

sender_key_state *create_test_sender_key_state(int id, int iteration)
{
    int result = 0;
    sender_key_state *state = 0;
    signal_buffer *buffer = 0;
    sender_chain_key *chain_key = 0;
    ec_key_pair *key_pair = 0;

    result = signal_protocol_key_helper_generate_sender_key(&buffer, global_context);
    ck_assert_int_eq(result, 0);

    result = sender_chain_key_create(&chain_key, iteration, buffer, global_context);
    ck_assert_int_eq(result, 0);

    result = signal_protocol_key_helper_generate_sender_signing_key(&key_pair, global_context);
    ck_assert_int_eq(result, 0);

    result = sender_key_state_create(&state, id, chain_key,
            ec_key_pair_get_public(key_pair), ec_key_pair_get_private(key_pair), global_context);
    ck_assert_int_eq(result, 0);

    /* Cleanup */
    signal_buffer_free(buffer);
    SIGNAL_UNREF(chain_key);
    SIGNAL_UNREF(key_pair);

    return state;
}

void compare_sender_chain_keys(sender_chain_key *chain_key1, sender_chain_key *chain_key2)
{
    int iteration1 = sender_chain_key_get_iteration(chain_key1);
    int iteration2 = sender_chain_key_get_iteration(chain_key2);
    ck_assert_int_eq(iteration1, iteration2);

    signal_buffer *seed1 = sender_chain_key_get_seed(chain_key1);
    signal_buffer *seed2 = sender_chain_key_get_seed(chain_key2);
    ck_assert_int_eq(signal_buffer_compare(seed1, seed2), 0);
}

void compare_sender_key_states(sender_key_state *state1, sender_key_state *state2)
{
    /* Compare key IDs */
    int key_id1 = sender_key_state_get_key_id(state1);
    int key_id2 = sender_key_state_get_key_id(state2);
    ck_assert_int_eq(key_id1, key_id2);

    /* Compare chain keys */
    sender_chain_key *chain_key1 = sender_key_state_get_chain_key(state1);
    sender_chain_key *chain_key2 = sender_key_state_get_chain_key(state2);
    compare_sender_chain_keys(chain_key1, chain_key2);

    /* Compare public signing keys */
    ec_public_key *key_public1 = sender_key_state_get_signing_key_public(state1);
    ec_public_key *key_public2 = sender_key_state_get_signing_key_public(state2);
    ck_assert_int_eq(ec_public_key_compare(key_public1, key_public2), 0);

    /* Compare private signing keys */
    ec_private_key *key_private1 = sender_key_state_get_signing_key_private(state1);
    ec_private_key *key_private2 = sender_key_state_get_signing_key_private(state2);
    ck_assert_int_eq(ec_private_key_compare(key_private1, key_private2), 0);

    /* Message keys are not compared here */
}

void compare_sender_message_keys(sender_message_key *message_key1, sender_message_key *message_key2)
{
    int iteration1 = sender_message_key_get_iteration(message_key1);
    int iteration2 = sender_message_key_get_iteration(message_key1);
    ck_assert_int_eq(iteration1, iteration2);

    signal_buffer *iv1 = sender_message_key_get_iv(message_key1);
    signal_buffer *iv2 = sender_message_key_get_iv(message_key2);
    ck_assert_int_eq(signal_buffer_compare(iv1, iv2), 0);

    signal_buffer *cipher_key1 = sender_message_key_get_cipher_key(message_key1);
    signal_buffer *cipher_key2 = sender_message_key_get_cipher_key(message_key2);
    ck_assert_int_eq(signal_buffer_compare(cipher_key1, cipher_key2), 0);

    signal_buffer *seed1 = sender_message_key_get_seed(message_key1);
    signal_buffer *seed2 = sender_message_key_get_seed(message_key2);
    ck_assert_int_eq(signal_buffer_compare(seed1, seed2), 0);
}

START_TEST(test_serialize_sender_key_state)
{
    int result = 0;
    sender_key_state *state = create_test_sender_key_state(1234, 1);
    sender_message_key *message_key = 0;

    /* Create and add a message key */
    result = sender_chain_key_create_message_key(sender_key_state_get_chain_key(state), &message_key);
    ck_assert_int_ge(result, 0);
    result = sender_key_state_add_sender_message_key(state, message_key);
    ck_assert_int_eq(result, 0);

    /* Serialize the state */
    signal_buffer *buffer = 0;
    result = sender_key_state_serialize(&buffer, state);
    ck_assert_int_ge(result, 0);

    /* Deserialize the state */
    uint8_t *data = signal_buffer_data(buffer);
    int len = signal_buffer_len(buffer);
    sender_key_state *state_deserialized = 0;
    result = sender_key_state_deserialize(&state_deserialized, data, len, global_context);
    ck_assert_int_eq(result, 0);

    /* Verify that the initial and deserialized states match */
    compare_sender_key_states(state, state_deserialized);

    /* Remove the message key from the deserialized state for comparison */
    sender_message_key *message_key_deserialized =
            sender_key_state_remove_sender_message_key(state_deserialized, 1);
    ck_assert_ptr_ne(message_key_deserialized, 0);

    /* Verify that the initial and deserialized message keys match */
    compare_sender_message_keys(message_key, message_key_deserialized);

    /* Cleanup */
    SIGNAL_UNREF(state);
    SIGNAL_UNREF(message_key);
    signal_buffer_free(buffer);
    SIGNAL_UNREF(state_deserialized);
    SIGNAL_UNREF(message_key_deserialized);
}
END_TEST

void compare_sender_key_records(sender_key_record *record1, sender_key_record *record2)
{
    int empty1 = sender_key_record_is_empty(record1);
    int empty2 = sender_key_record_is_empty(record2);
    ck_assert_int_eq(empty1, empty2);

    /* Sender key states not compared, since there is no way to iterate across them */
}

void compare_sender_key_record_states(sender_key_record *record1, sender_key_record *record2, int id)
{
    int result = 0;
    sender_key_state *state1 = 0;
    sender_key_state *state2 = 0;

    result = sender_key_record_get_sender_key_state_by_id(record1, &state1, id);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(state1, 0);

    result = sender_key_record_get_sender_key_state_by_id(record2, &state2, id);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(state2, 0);

    ck_assert_ptr_ne(state1, state2);

    compare_sender_key_states(state1, state2);
}

START_TEST(test_serialize_sender_key_record)
{
    int result = 0;
    sender_key_record *record = 0;

    result = sender_key_record_create(&record, global_context);
    ck_assert_int_eq(result, 0);

    /* Serialize the record */
    signal_buffer *buffer = 0;
    result = sender_key_record_serialize(&buffer, record);
    ck_assert_int_ge(result, 0);

    /* Deserialize the record */
    uint8_t *data = signal_buffer_data(buffer);
    int len = signal_buffer_len(buffer);
    sender_key_record *record_deserialized = 0;
    result = sender_key_record_deserialize(&record_deserialized, data, len, global_context);
    ck_assert_int_eq(result, 0);

    /* Verify that the initial and deserialized records match */
    compare_sender_key_records(record, record_deserialized);

    /* Cleanup */
    SIGNAL_UNREF(record);
    signal_buffer_free(buffer);
    SIGNAL_UNREF(record_deserialized);
}
END_TEST

START_TEST(test_serialize_sender_key_record_with_states)
{
    int result = 0;
    sender_key_record *record = 0;
    signal_buffer *buffer = 0;
    ec_key_pair *key_pair = 0;

    /* Create the record */
    result = sender_key_record_create(&record, global_context);
    ck_assert_int_eq(result, 0);

    /* Create and set state id=1000, iteration=1 */
    result = signal_protocol_key_helper_generate_sender_key(&buffer, global_context);
    ck_assert_int_eq(result, 0);
    result = signal_protocol_key_helper_generate_sender_signing_key(&key_pair, global_context);
    ck_assert_int_eq(result, 0);

    result = sender_key_record_set_sender_key_state(record, 1000, 1, buffer, key_pair);
    ck_assert_int_eq(result, 0);

    signal_buffer_free(buffer);
    SIGNAL_UNREF(key_pair);

    /* Create and add state id=1001, iteration=2 */
    result = signal_protocol_key_helper_generate_sender_key(&buffer, global_context);
    ck_assert_int_eq(result, 0);
    result = signal_protocol_key_helper_generate_sender_signing_key(&key_pair, global_context);
    ck_assert_int_eq(result, 0);

    sender_key_record_add_sender_key_state(record, 1001, 2, buffer, ec_key_pair_get_public(key_pair));
    ck_assert_int_eq(result, 0);

    signal_buffer_free(buffer);
    SIGNAL_UNREF(key_pair);

    /* Serialize the record */
    result = sender_key_record_serialize(&buffer, record);
    ck_assert_int_ge(result, 0);

    /* Deserialize the record */
    uint8_t *data = signal_buffer_data(buffer);
    int len = signal_buffer_len(buffer);
    sender_key_record *record_deserialized = 0;
    result = sender_key_record_deserialize(&record_deserialized, data, len, global_context);
    ck_assert_int_eq(result, 0);

    /* Verify that the initial and deserialized records match */
    compare_sender_key_records(record, record_deserialized);
    compare_sender_key_record_states(record, record_deserialized, 1000);
    compare_sender_key_record_states(record, record_deserialized, 1001);

    /* Cleanup */
    SIGNAL_UNREF(record);
    signal_buffer_free(buffer);
    SIGNAL_UNREF(record_deserialized);
}
END_TEST

START_TEST(test_sender_key_record_too_many_states)
{
    int result = 0;
    int i;
    sender_key_record *record = 0;
    sender_key_state *state = 0;
    signal_buffer *buffer = 0;
    ec_key_pair *key_pair = 0;

    /* Create the record */
    result = sender_key_record_create(&record, global_context);
    ck_assert_int_eq(result, 0);

    /* Create and set state id=1000, iteration=1 */
    result = signal_protocol_key_helper_generate_sender_key(&buffer, global_context);
    ck_assert_int_eq(result, 0);
    result = signal_protocol_key_helper_generate_sender_signing_key(&key_pair, global_context);
    ck_assert_int_eq(result, 0);

    result = sender_key_record_set_sender_key_state(record, 1000, 1, buffer, key_pair);
    ck_assert_int_eq(result, 0);

    signal_buffer_free(buffer);
    SIGNAL_UNREF(key_pair);

    /* Create and set states id=1001..1010, iteration=2..11 */
    for(i = 0; i < 10; i++) {
        result = signal_protocol_key_helper_generate_sender_key(&buffer, global_context);
        ck_assert_int_eq(result, 0);
        result = signal_protocol_key_helper_generate_sender_signing_key(&key_pair, global_context);
        ck_assert_int_eq(result, 0);

        sender_key_record_add_sender_key_state(record, 1001 + i, 2 + i, buffer, ec_key_pair_get_public(key_pair));
        ck_assert_int_eq(result, 0);

        signal_buffer_free(buffer);
        SIGNAL_UNREF(key_pair);
    }

    /* Get the latest state from the record */
    result = sender_key_record_get_sender_key_state(record, &state);
    ck_assert_int_eq(result, 0);
    ck_assert_int_eq(sender_key_state_get_key_id(state), 1010);

    /* Cleanup */
    SIGNAL_UNREF(record);
}
END_TEST

Suite *sender_key_record_suite(void)
{
    Suite *suite = suite_create("sender_key_record");

    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_serialize_sender_key_state);
    tcase_add_test(tcase, test_serialize_sender_key_record);
    tcase_add_test(tcase, test_serialize_sender_key_record_with_states);
    tcase_add_test(tcase, test_sender_key_record_too_many_states);
    suite_add_tcase(suite, tcase);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = sender_key_record_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
