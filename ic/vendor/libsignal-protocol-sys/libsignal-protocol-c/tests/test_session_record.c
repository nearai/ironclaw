#include <stdio.h>
#include <stdlib.h>
#include <check.h>

#include "../src/signal_protocol.h"
#include "hkdf.h"
#include "session_record.h"
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

void fill_test_session_state(session_state *state, ec_public_key *receiver_chain_ratchet_key1, ec_public_key *receiver_chain_ratchet_key2)
{
    int result = 0;
    hkdf_context *kdf = 0;

    result = hkdf_create(&kdf, 2, global_context);
    ck_assert_int_eq(result, 0);

    uint8_t keySeed[32];
    memset(keySeed, 0x42, sizeof(keySeed));

    /* Set the session version */
    session_state_set_session_version(state, 2);

    /* Set local and remote identity keys */
    ec_public_key *local_identity_key = create_test_ec_public_key(global_context);
    ec_public_key *remote_identity_key = create_test_ec_public_key(global_context);
    session_state_set_local_identity_key(state, local_identity_key);
    session_state_set_remote_identity_key(state, remote_identity_key);
    SIGNAL_UNREF(local_identity_key);
    SIGNAL_UNREF(remote_identity_key);

    /* Set the root key */
    ratchet_root_key *root_key;
    result = ratchet_root_key_create(&root_key, kdf, keySeed, sizeof(keySeed), global_context);
    ck_assert_int_eq(result, 0);

    session_state_set_root_key(state, root_key);
    SIGNAL_UNREF(root_key);

    /* Set the previous counter */
    session_state_set_previous_counter(state, 4);

    /* Set the sender chain */
    ec_key_pair *sender_ratchet_key_pair;
    result = curve_generate_key_pair(global_context, &sender_ratchet_key_pair);
    ck_assert_int_eq(result, 0);

    ratchet_chain_key *sender_chain_key;
    result = ratchet_chain_key_create(&sender_chain_key, kdf, keySeed, sizeof(keySeed), 0, global_context);
    ck_assert_int_eq(result, 0);

    session_state_set_sender_chain(state, sender_ratchet_key_pair, sender_chain_key);
    SIGNAL_UNREF(sender_ratchet_key_pair);
    SIGNAL_UNREF(sender_chain_key);

    /* Set the receiver chains */
    if(receiver_chain_ratchet_key1) {
        ratchet_chain_key *receiver_chain_chain_key1;
        result = ratchet_chain_key_create(&receiver_chain_chain_key1, kdf, keySeed, sizeof(keySeed), 0, global_context);
        ck_assert_int_eq(result, 0);

        result = session_state_add_receiver_chain(state, receiver_chain_ratchet_key1, receiver_chain_chain_key1);
        ck_assert_int_eq(result, 0);

        ratchet_message_keys message_keys;

        result = ratchet_chain_key_get_message_keys(receiver_chain_chain_key1, &message_keys);
        ck_assert_int_eq(result, 0);
        result = session_state_set_message_keys(state, receiver_chain_ratchet_key1, &message_keys);
        ck_assert_int_eq(result, 0);
        SIGNAL_UNREF(receiver_chain_chain_key1);
    }

    if(receiver_chain_ratchet_key2) {
        ratchet_chain_key *receiver_chain_chain_key2;
        result = ratchet_chain_key_create(&receiver_chain_chain_key2, kdf, keySeed, sizeof(keySeed), 0, global_context);
        ck_assert_int_eq(result, 0);

        result = session_state_add_receiver_chain(state, receiver_chain_ratchet_key2, receiver_chain_chain_key2);
        ck_assert_int_eq(result, 0);

        ratchet_message_keys message_keys;

        result = ratchet_chain_key_get_message_keys(receiver_chain_chain_key2, &message_keys);
        ck_assert_int_eq(result, 0);
        result = session_state_set_message_keys(state, receiver_chain_ratchet_key2, &message_keys);
        ck_assert_int_eq(result, 0);
        SIGNAL_UNREF(receiver_chain_chain_key2);
    }

    /* Set pending key exchange */
    ec_key_pair *our_base_key;
    result = curve_generate_key_pair(global_context, &our_base_key);
    ck_assert_int_eq(result, 0);

    ec_key_pair *our_ratchet_key;
    result = curve_generate_key_pair(global_context, &our_ratchet_key);
    ck_assert_int_eq(result, 0);

    ec_key_pair *our_identity_key_pair;
    result = curve_generate_key_pair(global_context, &our_identity_key_pair);
    ck_assert_int_eq(result, 0);

    ratchet_identity_key_pair *our_identity_key;
    result = ratchet_identity_key_pair_create(&our_identity_key,
            ec_key_pair_get_public(our_identity_key_pair),
            ec_key_pair_get_private(our_identity_key_pair));
    ck_assert_int_eq(result, 0);
    SIGNAL_UNREF(our_identity_key_pair);

    session_state_set_pending_key_exchange(state, 42,
            our_base_key, our_ratchet_key, our_identity_key);
    SIGNAL_UNREF(our_base_key);
    SIGNAL_UNREF(our_ratchet_key);
    SIGNAL_UNREF(our_identity_key);

    /* Set pending pre-key */
    ec_public_key *pending_pre_key_base_key = create_test_ec_public_key(global_context);
    uint32_t pre_key_id = 1234;
    session_state_set_unacknowledged_pre_key_message(state,
            &pre_key_id, 5678, pending_pre_key_base_key);
    SIGNAL_UNREF(pending_pre_key_base_key);

    session_state_set_remote_registration_id(state, 0xDEADBEEF);
    session_state_set_local_registration_id(state, 0xBAADF00D);

    session_state_set_needs_refresh(state, 0);

    ec_public_key *alice_base_key = create_test_ec_public_key(global_context);
    session_state_set_alice_base_key(state, alice_base_key);
    SIGNAL_UNREF(alice_base_key);

    SIGNAL_UNREF(kdf);
}

session_state *create_test_session_state(ec_public_key *receiver_chain_ratchet_key1, ec_public_key *receiver_chain_ratchet_key2)
{
    int result = 0;
    session_state *state = 0;

    result = session_state_create(&state, global_context);
    ck_assert_int_eq(result, 0);

    fill_test_session_state(state, receiver_chain_ratchet_key1, receiver_chain_ratchet_key2);

    return state;
}

void compare_session_states_receiver_chain(session_state *state1, session_state *state2, ec_public_key *sender_ephemeral);

void compare_session_states(session_state *state1, session_state *state2,
        ec_public_key *receiver_chain_ratchet_key1, ec_public_key *receiver_chain_ratchet_key2)
{
    int result = 0;

    /* Compare session versions */
    int version1 = session_state_get_session_version(state1);
    int version2 = session_state_get_session_version(state2);
    ck_assert_int_eq(version1, version2);

    /* Compare local identity keys */
    ec_public_key *local_identity_key1 = session_state_get_local_identity_key(state1);
    ec_public_key *local_identity_key2 = session_state_get_local_identity_key(state2);
    ck_assert_int_eq(ec_public_key_compare(local_identity_key1, local_identity_key2), 0);

    /* Compare remote identity keys */
    ec_public_key *remote_identity_key1 = session_state_get_remote_identity_key(state1);
    ec_public_key *remote_identity_key2 = session_state_get_remote_identity_key(state2);
    ck_assert_int_eq(ec_public_key_compare(remote_identity_key1, remote_identity_key2), 0);

    /* Compare root keys */
    ratchet_root_key *root_key1 = session_state_get_root_key(state1);
    ratchet_root_key *root_key2 = session_state_get_root_key(state2);
    ck_assert_int_eq(ratchet_root_key_compare(root_key1, root_key2), 0);

    /* Compare previous counters */
    int previous_counter1 = session_state_get_previous_counter(state1);
    int previous_counter2 = session_state_get_previous_counter(state2);
    ck_assert_int_eq(previous_counter1, previous_counter2);

    /* Compare sender chain */
    int has_sender_chain1 = session_state_has_sender_chain(state1);
    int has_sender_chain2 = session_state_has_sender_chain(state2);
    ck_assert_int_eq(has_sender_chain1, has_sender_chain2);

    if(has_sender_chain1 == 1) {
        /* Compare sender ratchet keys */
        ec_key_pair *sender_ratchet_key_pair1 = session_state_get_sender_ratchet_key_pair(state1);
        ec_key_pair *sender_ratchet_key_pair2 = session_state_get_sender_ratchet_key_pair(state2);

        ec_public_key *sender_ratchet_key_public1 = ec_key_pair_get_public(sender_ratchet_key_pair1);
        ec_public_key *sender_ratchet_key_public2 = ec_key_pair_get_public(sender_ratchet_key_pair2);
        ck_assert_int_eq(ec_public_key_compare(sender_ratchet_key_public1, sender_ratchet_key_public2), 0);

        ec_private_key *sender_ratchet_key_private1 = ec_key_pair_get_private(sender_ratchet_key_pair1);
        ec_private_key *sender_ratchet_key_private2 = ec_key_pair_get_private(sender_ratchet_key_pair2);
        ck_assert_int_eq(ec_private_key_compare(sender_ratchet_key_private1, sender_ratchet_key_private2), 0);

        /* Compare sender chain keys */
        ratchet_chain_key *sender_chain_key1 = session_state_get_sender_chain_key(state1);
        ratchet_chain_key *sender_chain_key2 = session_state_get_sender_chain_key(state2);

        signal_buffer *sender_chain_key_buf1;
        signal_buffer *sender_chain_key_buf2;
        result = ratchet_chain_key_get_key(sender_chain_key1, &sender_chain_key_buf1);
        ck_assert_int_eq(result, 0);
        result = ratchet_chain_key_get_key(sender_chain_key2, &sender_chain_key_buf2);
        ck_assert_int_eq(result, 0);

        ck_assert_int_eq(signal_buffer_compare(sender_chain_key_buf1, sender_chain_key_buf2), 0);
        signal_buffer_free(sender_chain_key_buf1);
        signal_buffer_free(sender_chain_key_buf2);

        int sender_chain_key_index1 = ratchet_chain_key_get_index(sender_chain_key1);
        int sender_chain_key_index2 = ratchet_chain_key_get_index(sender_chain_key2);
        ck_assert_int_eq(sender_chain_key_index1, sender_chain_key_index2);
    }

    /* Compare receiver chains */
    if(receiver_chain_ratchet_key1) {
        compare_session_states_receiver_chain(state1, state2, receiver_chain_ratchet_key1);
    }
    if(receiver_chain_ratchet_key2) {
        compare_session_states_receiver_chain(state1, state2, receiver_chain_ratchet_key2);
    }

    /* Compare pending key exchange */
    int has_pending_key_exchange1 = session_state_has_pending_key_exchange(state1);
    int has_pending_key_exchange2 = session_state_has_pending_key_exchange(state2);
    ck_assert_int_eq(has_pending_key_exchange1, has_pending_key_exchange2);

    if(has_pending_key_exchange1 == 1) {
        /* Compare sequence numbers */
        int sequence1 = session_state_get_pending_key_exchange_sequence(state1);
        int sequence2 = session_state_get_pending_key_exchange_sequence(state2);
        ck_assert_int_eq(sequence1, sequence2);

        /* Compare base keys */
        ec_key_pair *base_key1 = session_state_get_pending_key_exchange_base_key(state1);
        ec_key_pair *base_key2 = session_state_get_pending_key_exchange_base_key(state2);

        ec_public_key *base_key_public1 = ec_key_pair_get_public(base_key1);
        ec_public_key *base_key_public2 = ec_key_pair_get_public(base_key2);
        ck_assert_int_eq(ec_public_key_compare(base_key_public1, base_key_public2), 0);

        ec_private_key *base_key_private1 = ec_key_pair_get_private(base_key1);
        ec_private_key *base_key_private2 = ec_key_pair_get_private(base_key2);
        ck_assert_int_eq(ec_private_key_compare(base_key_private1, base_key_private2), 0);

        /* Compare ratchet keys */
        ec_key_pair *ratchet_key1 = session_state_get_pending_key_exchange_ratchet_key(state1);
        ec_key_pair *ratchet_key2 = session_state_get_pending_key_exchange_ratchet_key(state2);

        ec_public_key *ratchet_key_public1 = ec_key_pair_get_public(ratchet_key1);
        ec_public_key *ratchet_key_public2 = ec_key_pair_get_public(ratchet_key2);
        ck_assert_int_eq(ec_public_key_compare(ratchet_key_public1, ratchet_key_public2), 0);

        ec_private_key *ratchet_key_private1 = ec_key_pair_get_private(ratchet_key1);
        ec_private_key *ratchet_key_private2 = ec_key_pair_get_private(ratchet_key2);
        ck_assert_int_eq(ec_private_key_compare(ratchet_key_private1, ratchet_key_private2), 0);

        /* Compare identity keys */
        ratchet_identity_key_pair *identity_key1 = session_state_get_pending_key_exchange_identity_key(state1);
        ratchet_identity_key_pair *identity_key2 = session_state_get_pending_key_exchange_identity_key(state2);

        ec_public_key *identity_key_public1 = ratchet_identity_key_pair_get_public(identity_key1);
        ec_public_key *identity_key_public2 = ratchet_identity_key_pair_get_public(identity_key2);
        ck_assert_int_eq(ec_public_key_compare(identity_key_public1, identity_key_public2), 0);

        ec_private_key *identity_key_private1 = ratchet_identity_key_pair_get_private(identity_key1);
        ec_private_key *identity_key_private2 = ratchet_identity_key_pair_get_private(identity_key2);
        ck_assert_int_eq(ec_private_key_compare(identity_key_private1, identity_key_private2), 0);
    }

    /* Compare pending pre-key */
    int has_pending_pre_key1 = session_state_has_unacknowledged_pre_key_message(state1);
    int has_pending_pre_key2 = session_state_has_unacknowledged_pre_key_message(state2);
    ck_assert_int_eq(has_pending_pre_key1, has_pending_pre_key2);

    if(has_pending_pre_key1 == 1) {
        int has_key_id1 = session_state_unacknowledged_pre_key_message_has_pre_key_id(state1);
        int has_key_id2 = session_state_unacknowledged_pre_key_message_has_pre_key_id(state2);
        ck_assert_int_eq(has_key_id1, has_key_id2);

        if(has_key_id1) {
            /* Compare pre-key IDs */
            uint32_t pre_key_id1 = session_state_unacknowledged_pre_key_message_get_pre_key_id(state1);
            uint32_t pre_key_id2 = session_state_unacknowledged_pre_key_message_get_pre_key_id(state2);
            ck_assert_int_eq(pre_key_id1, pre_key_id2);

            /* Compare signed pre-key IDs */
            int32_t signed_pre_key_id1 = session_state_unacknowledged_pre_key_message_get_signed_pre_key_id(state1);
            int32_t signed_pre_key_id2 = session_state_unacknowledged_pre_key_message_get_signed_pre_key_id(state2);
            ck_assert_int_eq(signed_pre_key_id1, signed_pre_key_id2);

            /* Compare base keys */
            ec_public_key *base_key1 = session_state_unacknowledged_pre_key_message_get_base_key(state1);
            ec_public_key *base_key2 = session_state_unacknowledged_pre_key_message_get_base_key(state2);
            ck_assert_int_eq(ec_public_key_compare(base_key1, base_key2), 0);
        }
    }

    /* Compare remote registration IDs */
    uint32_t remote_registration_id1 = session_state_get_remote_registration_id(state1);
    uint32_t remote_registration_id2 = session_state_get_remote_registration_id(state2);
    ck_assert_int_eq(remote_registration_id1, remote_registration_id2);

    /* Compare local registration IDs */
    uint32_t local_registration_id1 = session_state_get_local_registration_id(state1);
    uint32_t local_registration_id2 = session_state_get_local_registration_id(state2);
    ck_assert_int_eq(local_registration_id1, local_registration_id2);

    /* Compare refresh flags */
    int needs_refresh1 = session_state_get_needs_refresh(state1);
    int needs_refresh2 = session_state_get_needs_refresh(state2);
    ck_assert_int_eq(needs_refresh1, needs_refresh2);

    /* Compare Alice base keys */
    ec_public_key *alice_base_key1 = session_state_get_alice_base_key(state1);
    ec_public_key *alice_base_key2 = session_state_get_alice_base_key(state2);
    ck_assert_int_eq(ec_public_key_compare(alice_base_key1, alice_base_key2), 0);
}

void compare_session_states_receiver_chain(session_state *state1, session_state *state2, ec_public_key *sender_ephemeral)
{
    int result = 0;
    ratchet_chain_key *receiver_chain_key1 = session_state_get_receiver_chain_key(state1, sender_ephemeral);
    ck_assert_ptr_ne(receiver_chain_key1, 0);
    ratchet_chain_key *receiver_chain_key2 = session_state_get_receiver_chain_key(state2, sender_ephemeral);
    ck_assert_ptr_ne(receiver_chain_key2, 0);

    ck_assert_ptr_ne(receiver_chain_key1, receiver_chain_key2);

    signal_buffer *receiver_chain_key_buf1;
    signal_buffer *receiver_chain_key_buf2;
    result = ratchet_chain_key_get_key(receiver_chain_key1, &receiver_chain_key_buf1);
    ck_assert_int_eq(result, 0);
    result = ratchet_chain_key_get_key(receiver_chain_key2, &receiver_chain_key_buf2);
    ck_assert_int_eq(result, 0);

    ck_assert_int_eq(signal_buffer_compare(receiver_chain_key_buf1, receiver_chain_key_buf2), 0);
    signal_buffer_free(receiver_chain_key_buf1);
    signal_buffer_free(receiver_chain_key_buf2);

    int receiver_chain_key_index1 = ratchet_chain_key_get_index(receiver_chain_key1);
    int receiver_chain_key_index2 = ratchet_chain_key_get_index(receiver_chain_key2);
    ck_assert_int_eq(receiver_chain_key_index1, receiver_chain_key_index2);

    int has_message_keys1 = session_state_has_message_keys(state1, sender_ephemeral, 0);
    ck_assert_int_eq(has_message_keys1, 1);
    int has_message_keys2 = session_state_has_message_keys(state2, sender_ephemeral, 0);
    ck_assert_int_eq(has_message_keys2, 1);

    ratchet_message_keys message_keys1;
    result = session_state_remove_message_keys(state1, &message_keys1, sender_ephemeral, 0);
    ck_assert_int_eq(result, 1);

    ratchet_message_keys message_keys2;
    result = session_state_remove_message_keys(state2, &message_keys2, sender_ephemeral, 0);
    ck_assert_int_eq(result, 1);

    ck_assert_int_eq(memcmp(&message_keys1, &message_keys2, sizeof(ratchet_message_keys)), 0);
}

START_TEST(test_serialize_single_session)
{
    int result = 0;
    ec_public_key *receiver_chain_ratchet_key1 = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key2 = create_test_ec_public_key(global_context);
    session_state *state = create_test_session_state(receiver_chain_ratchet_key1, receiver_chain_ratchet_key2);

    session_record *record = 0;
    result = session_record_create(&record, state, global_context);
    ck_assert_int_eq(result, 0);

    signal_buffer *buffer = 0;
    result = session_record_serialize(&buffer, record);
    ck_assert_int_ge(result, 0);

    uint8_t *data = signal_buffer_data(buffer);
    int len = signal_buffer_len(buffer);

    /* Deserialize the record */
    session_record *record_deserialized = 0;
    result = session_record_deserialize(&record_deserialized, data, len, global_context);
    ck_assert_int_ge(result, 0);

    session_state *state_deserialized = session_record_get_state(record_deserialized);
    ck_assert_ptr_ne(state_deserialized, 0);
    ck_assert_ptr_ne(state_deserialized, state);

    /* Verify that the initial and deserialized states match */
    compare_session_states(state, state_deserialized, receiver_chain_ratchet_key1, receiver_chain_ratchet_key2);

    /* Verify that there aren't any previous states */
    session_record_state_node *previous_node = session_record_get_previous_states_head(record_deserialized);
    ck_assert_ptr_eq(previous_node, 0);

    /* Cleanup */
    SIGNAL_UNREF(state);
    signal_buffer_free(buffer);
    SIGNAL_UNREF(receiver_chain_ratchet_key1);
    SIGNAL_UNREF(receiver_chain_ratchet_key2);
    SIGNAL_UNREF(record);
    SIGNAL_UNREF(record_deserialized);
}
END_TEST

START_TEST(test_serialize_multiple_sessions)
{
    int result = 0;
    /* Create several test keys */
    ec_public_key *receiver_chain_ratchet_key1a = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key1b = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key2a = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key2b = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key3a = create_test_ec_public_key(global_context);
    ec_public_key *receiver_chain_ratchet_key3b = create_test_ec_public_key(global_context);

    /* Create the session record with the first state */
    session_state *state1 = create_test_session_state(receiver_chain_ratchet_key1a, receiver_chain_ratchet_key1b);
    session_record *record = 0;
    result = session_record_create(&record, state1, global_context);
    ck_assert_int_eq(result, 0);

    /* Archive the current state and fill the new state */
    result = session_record_archive_current_state(record);
    ck_assert_int_eq(result, 0);
    session_state *state2 = session_record_get_state(record);
    ck_assert_ptr_ne(state1, state2);
    fill_test_session_state(state2, receiver_chain_ratchet_key2a, receiver_chain_ratchet_key2b);

    /* Archive the current state and fill the new state */
    result = session_record_archive_current_state(record);
    ck_assert_int_eq(result, 0);
    session_state *state3 = session_record_get_state(record);
    ck_assert_ptr_ne(state1, state2);
    ck_assert_ptr_ne(state1, state3);
    fill_test_session_state(state3, receiver_chain_ratchet_key3a, receiver_chain_ratchet_key3b);

    signal_buffer *buffer = 0;
    result = session_record_serialize(&buffer, record);
    ck_assert_int_ge(result, 0);

    uint8_t *data = signal_buffer_data(buffer);
    int len = signal_buffer_len(buffer);

    /* Deserialize the record */
    session_record *record_deserialized = 0;
    result = session_record_deserialize(&record_deserialized, data, len, global_context);
    ck_assert_int_ge(result, 0);

    session_state *state_deserialized3 = session_record_get_state(record_deserialized);
    ck_assert_ptr_ne(state_deserialized3, 0);
    ck_assert_ptr_ne(state_deserialized3, state3);

    /* Verify that the expected and actual current states match */
    compare_session_states(state3, state_deserialized3, receiver_chain_ratchet_key3a, receiver_chain_ratchet_key3b);

    /* Verify that we have the expected number of previous states */
    session_record_state_node *previous_node = session_record_get_previous_states_head(record_deserialized);
    ck_assert_ptr_ne(previous_node, 0);

    session_state *state_deserialized2 = session_record_get_previous_states_element(previous_node);
    ck_assert_ptr_ne(state_deserialized2, 0);

    previous_node = session_record_get_previous_states_next(previous_node);
    ck_assert_ptr_ne(previous_node, 0);

    session_state *state_deserialized1 = session_record_get_previous_states_element(previous_node);
    ck_assert_ptr_ne(state_deserialized1, 0);

    previous_node = session_record_get_previous_states_next(previous_node);
    ck_assert_ptr_eq(previous_node, 0);

    /* Verify that the expected and actual previous states match */
    compare_session_states(state2, state_deserialized2, receiver_chain_ratchet_key2a, receiver_chain_ratchet_key2b);
    compare_session_states(state1, state_deserialized1, receiver_chain_ratchet_key1a, receiver_chain_ratchet_key1b);

    /* Cleanup */
    SIGNAL_UNREF(state1);
    signal_buffer_free(buffer);
    SIGNAL_UNREF(receiver_chain_ratchet_key1a);
    SIGNAL_UNREF(receiver_chain_ratchet_key1b);
    SIGNAL_UNREF(receiver_chain_ratchet_key2a);
    SIGNAL_UNREF(receiver_chain_ratchet_key2b);
    SIGNAL_UNREF(receiver_chain_ratchet_key3a);
    SIGNAL_UNREF(receiver_chain_ratchet_key3b);
    SIGNAL_UNREF(record);
    SIGNAL_UNREF(record_deserialized);
}
END_TEST

START_TEST(test_session_receiver_chain_count)
{
    int result = 0;
    int i = 0;
    hkdf_context *kdf = 0;
    session_state *state = 0;
    ratchet_chain_key *chain_key[7];
    ec_public_key *ratchet_key[7];

    result = hkdf_create(&kdf, 2, global_context);
    ck_assert_int_eq(result, 0);

    uint8_t keySeed[32];
    memset(keySeed, 0x42, sizeof(keySeed));

    /* Create 7 instances of receiver chain data */
    for(i = 0; i < 7; i++) {
        result = ratchet_chain_key_create(&chain_key[i], kdf, keySeed, sizeof(keySeed), 0, global_context);
        ck_assert_int_eq(result, 0);

        ratchet_key[i] = create_test_ec_public_key(global_context);
        ck_assert_ptr_ne(ratchet_key[i], 0);
    }

    /* Create a new session state instance */
    result = session_state_create(&state, global_context);
    ck_assert_int_eq(result, 0);


    /* Add 6 instances of receiver chain data */
    for(i = 0; i < 7; i++) {
        result = session_state_add_receiver_chain(state, ratchet_key[i], chain_key[i]);
        ck_assert_int_eq(result, 0);
    }

    /* Verify that only the latter 5 are actually there */
    for(i = 0; i < 7; i++) {
        ratchet_chain_key *cur_chain_key;
        signal_buffer *chain_key_buf;
        signal_buffer *cur_chain_key_buf;

        cur_chain_key = session_state_get_receiver_chain_key(state, ratchet_key[i]);

        if(i < 2) {
            ck_assert_ptr_eq(cur_chain_key, 0);
        }
        else {
            ck_assert_ptr_ne(cur_chain_key, 0);

            result = ratchet_chain_key_get_key(chain_key[i], &chain_key_buf);
            ck_assert_int_eq(result, 0);
            result = ratchet_chain_key_get_key(cur_chain_key, &cur_chain_key_buf);
            ck_assert_int_eq(result, 0);

            ck_assert_int_eq(signal_buffer_compare(chain_key_buf, cur_chain_key_buf), 0);
            signal_buffer_free(chain_key_buf);
            signal_buffer_free(cur_chain_key_buf);
        }
    }

    /* Cleanup */
    for(i = 0; i < 7; i++) {
        SIGNAL_UNREF(chain_key[i]);
        SIGNAL_UNREF(ratchet_key[i]);
    }
    SIGNAL_UNREF(kdf);
    SIGNAL_UNREF(state);
}
END_TEST

Suite *session_record_suite(void)
{
    Suite *suite = suite_create("session_record");

    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_serialize_single_session);
    tcase_add_test(tcase, test_serialize_multiple_sessions);
    tcase_add_test(tcase, test_session_receiver_chain_count);
    suite_add_tcase(suite, tcase);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = session_record_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
