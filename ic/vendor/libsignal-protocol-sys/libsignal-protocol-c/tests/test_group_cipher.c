#include <stdio.h>
#include <stdlib.h>
#include <check.h>
#include <pthread.h>

#include "../src/signal_protocol.h"
#include "protocol.h"
#include "group_cipher.h"
#include "group_session_builder.h"
#include "test_common.h"
#include "test_utarray.h"

signal_context *global_context;
pthread_mutex_t global_mutex;
pthread_mutexattr_t global_mutex_attr;

static signal_protocol_sender_key_name GROUP_SENDER = {
        "nihilist history reading group", 30,
        {"+14150001111", 12, 1}
};

void test_lock(void *user_data)
{
    pthread_mutex_lock(&global_mutex);
}

void test_unlock(void *user_data)
{
    pthread_mutex_unlock(&global_mutex);
}

void test_setup()
{
    int result;

    pthread_mutexattr_init(&global_mutex_attr);
    pthread_mutexattr_settype(&global_mutex_attr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&global_mutex, &global_mutex_attr);

    result = signal_context_create(&global_context, 0);
    ck_assert_int_eq(result, 0);
    signal_context_set_log_function(global_context, test_log);

    setup_test_crypto_provider(global_context);

    result = signal_context_set_locking_functions(global_context, test_lock, test_unlock);
    ck_assert_int_eq(result, 0);
}

void test_teardown()
{
    signal_context_destroy(global_context);
}

START_TEST(test_no_session)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builder */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, &GROUP_SENDER, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, &GROUP_SENDER, global_context);

    /* Create the sender key distribution messages */
    sender_key_distribution_message *sent_alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &sent_alice_distribution_message, &GROUP_SENDER);
    ck_assert_int_eq(result, 0);

    sender_key_distribution_message *received_alice_distribution_message = 0;
    signal_buffer *serialized_distribution_message =
            ciphertext_message_get_serialized((ciphertext_message *)sent_alice_distribution_message);
    result = sender_key_distribution_message_deserialize(&received_alice_distribution_message,
            signal_buffer_data(serialized_distribution_message),
            signal_buffer_len(serialized_distribution_message),
            global_context);
    ck_assert_int_eq(result, 0);

    /* Intentionally omitting Bob's processing of received_alice_distribution_message */

    /* Encrypt a test message from Alice */
    static const char alice_plaintext[] = "smert ze smert";
    size_t alice_plaintext_len = sizeof(alice_plaintext) - 1;
    ciphertext_message *ciphertext_from_alice = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)alice_plaintext, alice_plaintext_len,
            &ciphertext_from_alice);
    ck_assert_int_eq(result, 0);

    /* Attempt to have Bob decrypt the message */
    signal_buffer *plaintext_from_alice = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice, 0, &plaintext_from_alice);
    ck_assert_int_eq(result, SG_ERR_NO_SESSION);;

    /* Cleanup */
    signal_buffer_free(plaintext_from_alice);
    SIGNAL_UNREF(ciphertext_from_alice);
    SIGNAL_UNREF(received_alice_distribution_message);
    SIGNAL_UNREF(sent_alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_basic_encrypt_decrypt)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, &GROUP_SENDER, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, &GROUP_SENDER, global_context);

    /* Create the sender key distribution messages */
    sender_key_distribution_message *sent_alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &sent_alice_distribution_message, &GROUP_SENDER);
    ck_assert_int_eq(result, 0);

    sender_key_distribution_message *received_alice_distribution_message = 0;
    signal_buffer *serialized_distribution_message =
            ciphertext_message_get_serialized((ciphertext_message *)sent_alice_distribution_message);
    result = sender_key_distribution_message_deserialize(&received_alice_distribution_message,
            signal_buffer_data(serialized_distribution_message),
            signal_buffer_len(serialized_distribution_message),
            global_context);
    ck_assert_int_eq(result, 0);

    /* Processing Alice's distribution message */
    result = group_session_builder_process_session(bob_session_builder, &GROUP_SENDER, received_alice_distribution_message);
    ck_assert_int_eq(result, 0);

    /* Encrypt a test message from Alice */
    static const char alice_plaintext[] = "smert ze smert";
    size_t alice_plaintext_len = sizeof(alice_plaintext) - 1;
    ciphertext_message *ciphertext_from_alice = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)alice_plaintext, alice_plaintext_len,
            &ciphertext_from_alice);
    ck_assert_int_eq(result, 0);

    /* Have Bob decrypt the message */
    signal_buffer *plaintext_from_alice = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice, 0, &plaintext_from_alice);
    ck_assert_int_eq(result, 0);

    uint8_t *plaintext_data = signal_buffer_data(plaintext_from_alice);
    size_t plaintext_len = signal_buffer_len(plaintext_from_alice);

    ck_assert_int_eq(alice_plaintext_len, plaintext_len);
    ck_assert_int_eq(memcmp(alice_plaintext, plaintext_data, plaintext_len), 0);

    /* Cleanup */
    signal_buffer_free(plaintext_from_alice);
    SIGNAL_UNREF(ciphertext_from_alice);
    SIGNAL_UNREF(received_alice_distribution_message);
    SIGNAL_UNREF(sent_alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_basic_ratchet)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    signal_protocol_sender_key_name *alice_name = &GROUP_SENDER;

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, alice_name, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, alice_name, global_context);

    /* Create the sender key distribution messages */
    sender_key_distribution_message *sent_alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &sent_alice_distribution_message, alice_name);
    ck_assert_int_eq(result, 0);

    sender_key_distribution_message *received_alice_distribution_message = 0;
    signal_buffer *serialized_distribution_message =
            ciphertext_message_get_serialized((ciphertext_message *)sent_alice_distribution_message);
    result = sender_key_distribution_message_deserialize(&received_alice_distribution_message,
            signal_buffer_data(serialized_distribution_message),
            signal_buffer_len(serialized_distribution_message),
            global_context);
    ck_assert_int_eq(result, 0);

    /* Processing Alice's distribution message */
    result = group_session_builder_process_session(bob_session_builder, alice_name, received_alice_distribution_message);
    ck_assert_int_eq(result, 0);

    /* Prepare some text to encrypt */
    static const char alice_plaintext[] = "smert ze smert";
    size_t alice_plaintext_len = sizeof(alice_plaintext) - 1;
    static const char alice_plaintext_2[] = "smert ze smert2";
    size_t alice_plaintext_2_len = sizeof(alice_plaintext_2) - 1;
    static const char alice_plaintext_3[] = "smert ze smert3";
    size_t alice_plaintext_3_len = sizeof(alice_plaintext_3) - 1;

    /* Encrypt a series of messages from Alice */
    ciphertext_message *ciphertext_from_alice = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)alice_plaintext, alice_plaintext_len,
            &ciphertext_from_alice);
    ck_assert_int_eq(result, 0);

    ciphertext_message *ciphertext_from_alice_2 = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)alice_plaintext_2, alice_plaintext_2_len,
            &ciphertext_from_alice_2);
    ck_assert_int_eq(result, 0);

    ciphertext_message *ciphertext_from_alice_3 = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)alice_plaintext_3, alice_plaintext_3_len,
            &ciphertext_from_alice_3);
    ck_assert_int_eq(result, 0);

    /* Have Bob decrypt the message */
    signal_buffer *plaintext_from_alice = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice, 0, &plaintext_from_alice);
    ck_assert_int_eq(result, 0);

    /* Have Bob attempt to decrypt the same message again */
    signal_buffer *plaintext_from_alice_repeat = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice, 0, &plaintext_from_alice_repeat);
    ck_assert_int_eq(result, SG_ERR_DUPLICATE_MESSAGE); /* Should have ratcheted forward */
    ck_assert_ptr_eq(plaintext_from_alice_repeat, 0);

    /* Have Bob decrypt the remaining messages */
    signal_buffer *plaintext_from_alice_2 = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice_2, 0, &plaintext_from_alice_2);
    ck_assert_int_eq(result, 0);

    signal_buffer *plaintext_from_alice_3 = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)ciphertext_from_alice_3, 0, &plaintext_from_alice_3);
    ck_assert_int_eq(result, 0);

    /* Verify that the plaintext matches */
    uint8_t *plaintext_data = signal_buffer_data(plaintext_from_alice);
    size_t plaintext_len = signal_buffer_len(plaintext_from_alice);
    ck_assert_int_eq(alice_plaintext_len, plaintext_len);
    ck_assert_int_eq(memcmp(alice_plaintext, plaintext_data, plaintext_len), 0);

    plaintext_data = signal_buffer_data(plaintext_from_alice_2);
    plaintext_len = signal_buffer_len(plaintext_from_alice_2);
    ck_assert_int_eq(alice_plaintext_2_len, plaintext_len);
    ck_assert_int_eq(memcmp(alice_plaintext_2, plaintext_data, plaintext_len), 0);

    plaintext_data = signal_buffer_data(plaintext_from_alice_3);
    plaintext_len = signal_buffer_len(plaintext_from_alice_3);
    ck_assert_int_eq(alice_plaintext_3_len, plaintext_len);
    ck_assert_int_eq(memcmp(alice_plaintext_3, plaintext_data, plaintext_len), 0);

    /* Cleanup */
    signal_buffer_free(plaintext_from_alice_3);
    signal_buffer_free(plaintext_from_alice_2);
    signal_buffer_free(plaintext_from_alice);
    SIGNAL_UNREF(ciphertext_from_alice_3);
    SIGNAL_UNREF(ciphertext_from_alice_2);
    SIGNAL_UNREF(ciphertext_from_alice);
    SIGNAL_UNREF(received_alice_distribution_message);
    SIGNAL_UNREF(sent_alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_late_join)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create Alice's session builder */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    signal_protocol_sender_key_name *alice_name = &GROUP_SENDER;

    /* Create Alice's group cipher */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, alice_name, global_context);

    /* Create Alice's sender key distribution message */
    sender_key_distribution_message *alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &alice_distribution_message, alice_name);
    ck_assert_int_eq(result, 0);
    /* Pretend this was sent to some people other than Bob */

    /* Encrypt a batch of messages that Bob never receives */
    int i = 0;
    for(i = 0; i < 100; i++) {
        static const char alice_plaintext[] = "up the punks up the punks up the punks";
        size_t alice_plaintext_len = sizeof(alice_plaintext) - 1;

        ciphertext_message *ciphertext_from_alice = 0;
        result = group_cipher_encrypt(alice_group_cipher,
                (const uint8_t *)alice_plaintext, alice_plaintext_len,
                &ciphertext_from_alice);
        ck_assert_int_eq(result, 0);
        SIGNAL_UNREF(ciphertext_from_alice);
    }

    /* Now Bob Joins */
    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, alice_name, global_context);

    /* Create Alice's sender key distribution message for Bob */
    sender_key_distribution_message *distribution_message_to_bob = 0;
    result = group_session_builder_create_session(alice_session_builder, &distribution_message_to_bob, alice_name);
    ck_assert_int_eq(result, 0);

    sender_key_distribution_message *received_distribution_message_to_bob = 0;
    signal_buffer *serialized_distribution_message =
            ciphertext_message_get_serialized((ciphertext_message *)distribution_message_to_bob);
    result = sender_key_distribution_message_deserialize(&received_distribution_message_to_bob,
            signal_buffer_data(serialized_distribution_message),
            signal_buffer_len(serialized_distribution_message),
            global_context);
    ck_assert_int_eq(result, 0);

    /* Have Bob process Alice's distribution message */
    result = group_session_builder_process_session(bob_session_builder, alice_name, received_distribution_message_to_bob);
    ck_assert_int_eq(result, 0);

    /* Alice sends a message welcoming Bob */
    static const char welcome_plaintext[] = "welcome to the group";
    size_t welcome_plaintext_len = sizeof(welcome_plaintext) - 1;

    ciphertext_message *ciphertext = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)welcome_plaintext, welcome_plaintext_len,
            &ciphertext);
    ck_assert_int_eq(result, 0);

    /* Bob decrypts the message */
    signal_buffer *plaintext_from_alice = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message*)ciphertext, 0, &plaintext_from_alice);
    ck_assert_int_eq(result, 0);

    /* Verify that the plaintext matches */
    uint8_t *plaintext_data = signal_buffer_data(plaintext_from_alice);
    size_t plaintext_len = signal_buffer_len(plaintext_from_alice);
    ck_assert_int_eq(welcome_plaintext_len, plaintext_len);
    ck_assert_int_eq(memcmp(welcome_plaintext, plaintext_data, plaintext_len), 0);

    /* Cleanup */
    signal_buffer_free(plaintext_from_alice);
    SIGNAL_UNREF(ciphertext);
    SIGNAL_UNREF(received_distribution_message_to_bob);
    SIGNAL_UNREF(distribution_message_to_bob);
    group_cipher_free(bob_group_cipher);
    group_session_builder_free(bob_session_builder);
    SIGNAL_UNREF(alice_distribution_message);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_out_of_order)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    signal_protocol_sender_key_name *alice_name = &GROUP_SENDER;

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, alice_name, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, alice_name, global_context);

    /* Create Alice's sender key distribution message */
    sender_key_distribution_message *alice_distribution_message = 0;
    result = group_session_builder_create_session(
            alice_session_builder, &alice_distribution_message, alice_name);
    ck_assert_int_eq(result, 0);

    /* Have Bob process the distribution message */
    result = group_session_builder_process_session(bob_session_builder, alice_name, alice_distribution_message);
    ck_assert_int_eq(result, 0);

    /* Populate a batch of 100 messages */
    UT_array *ciphertexts;
    utarray_new(ciphertexts, &ut_ptr_icd);
    utarray_reserve(ciphertexts, 100);

    static const char plaintext[] = "up the punks";
    size_t plaintext_len = sizeof(plaintext) - 1;
    int i = 0;
    for(i = 0; i < 100; i++) {
        ciphertext_message *ciphertext = 0;
        result = group_cipher_encrypt(alice_group_cipher,
                (const uint8_t *)plaintext, plaintext_len,
                &ciphertext);
        ck_assert_int_eq(result, 0);

        signal_buffer *serialized = ciphertext_message_get_serialized(ciphertext);
        signal_buffer *serialized_copy = signal_buffer_copy(serialized);
        utarray_push_back(ciphertexts, &serialized_copy);
        SIGNAL_UNREF(ciphertext);
    }

    /* Try decrypting those messages in random order */
    while(utarray_len(ciphertexts) > 0) {
        /* Get the next element */
        int index = rand() % utarray_len(ciphertexts);
        signal_buffer *element = *((signal_buffer **)utarray_eltptr(ciphertexts, index));
        utarray_erase(ciphertexts, index, 1);

        /* Deserialize the message */
        sender_key_message *ciphertext = 0;
        result = sender_key_message_deserialize(&ciphertext,
                signal_buffer_data(element), signal_buffer_len(element),
                global_context);
        ck_assert_int_eq(result, 0);

        /* Decrypt the message */
        signal_buffer *plaintext_buffer = 0;
        result = group_cipher_decrypt(bob_group_cipher, ciphertext, 0, &plaintext_buffer);
        ck_assert_int_eq(result, 0);

        /* Verify that the plaintext matches */
        uint8_t *decrypted_plaintext_data = signal_buffer_data(plaintext_buffer);
        size_t decrypted_plaintext_len = signal_buffer_len(plaintext_buffer);
        ck_assert_int_eq(plaintext_len, decrypted_plaintext_len);
        ck_assert_int_eq(memcmp(plaintext, decrypted_plaintext_data, decrypted_plaintext_len), 0);

        signal_buffer_free(element);
        signal_buffer_free(plaintext_buffer);
        SIGNAL_UNREF(ciphertext);
    }

    /* Cleanup */
    utarray_free(ciphertexts);
    SIGNAL_UNREF(alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_encrypt_no_session)
{
    int result = 0;
    static const signal_protocol_sender_key_name alice_sender_name = {
            "coolio groupio", 14,
            {"+10002223333", 12, 1}
    };

    /* Create the test data store for Alice */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    /* Create Alice's group cipher */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, &alice_sender_name, global_context);
    ck_assert_int_eq(result, 0);

    /* Try to encrypt without a session */
    static const char plaintext[] = "up the punks";
    size_t plaintext_len = sizeof(plaintext) - 1;

    ciphertext_message *ciphertext = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)plaintext, plaintext_len,
            &ciphertext);
    ck_assert_int_eq(result, SG_ERR_NO_SESSION);
    ck_assert_ptr_eq(ciphertext, 0);

    /* Cleanup */
    group_cipher_free(alice_group_cipher);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_too_far_in_future)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    signal_protocol_sender_key_name *alice_name = &GROUP_SENDER;

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, alice_name, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, alice_name, global_context);

    /* Create Alice's sender key distribution message */
    sender_key_distribution_message *alice_distribution_message = 0;
    result = group_session_builder_create_session(
            alice_session_builder, &alice_distribution_message, alice_name);
    ck_assert_int_eq(result, 0);

    /* Have Bob process the distribution message */
    result = group_session_builder_process_session(bob_session_builder, alice_name, alice_distribution_message);
    ck_assert_int_eq(result, 0);

    /* Have Alice encrypt a batch of 2001 messages */
    static const char plaintext[] = "up the punks";
    size_t plaintext_len = sizeof(plaintext) - 1;
    int i = 0;
    for(i = 0; i < 2001; i++) {
        ciphertext_message *ciphertext = 0;
        result = group_cipher_encrypt(alice_group_cipher,
                (const uint8_t *)plaintext, plaintext_len,
                &ciphertext);
        ck_assert_int_eq(result, 0);
        SIGNAL_UNREF(ciphertext);
    }

    /* Have Alice encrypt a message too far in the future */
    static const char too_far_plaintext[] = "notta gonna worka";
    size_t too_far_plaintext_len = sizeof(too_far_plaintext) - 1;

    ciphertext_message *too_far_ciphertext = 0;
    result = group_cipher_encrypt(alice_group_cipher,
            (const uint8_t *)too_far_plaintext, too_far_plaintext_len,
            &too_far_ciphertext);
    ck_assert_int_eq(result, 0);

    /* Have Bob try, and fail, to decrypt the message */
    signal_buffer *plaintext_from_alice = 0;
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message*)too_far_ciphertext, 0, &plaintext_from_alice);
    ck_assert_int_eq(result, SG_ERR_INVALID_MESSAGE);

    /* Cleanup */
    SIGNAL_UNREF(too_far_ciphertext);
    SIGNAL_UNREF(alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_message_key_limit)
{
    int result = 0;
    int i;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    signal_protocol_sender_key_name *alice_name = &GROUP_SENDER;

    /* Create the group ciphers */
    group_cipher *alice_group_cipher = 0;
    result = group_cipher_create(&alice_group_cipher, alice_store, alice_name, global_context);

    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, alice_name, global_context);

    /* Create the sender key distribution messages */
    sender_key_distribution_message *alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &alice_distribution_message, alice_name);
    ck_assert_int_eq(result, 0);

    /* Processing Alice's distribution message */
    result = group_session_builder_process_session(bob_session_builder, alice_name, alice_distribution_message);
    ck_assert_int_eq(result, 0);

    ciphertext_message *inflight[2010];
    memset(inflight, 0, sizeof(inflight));

    for(i = 0; i <2010; i++) {
        static const char plaintext[] = "up the punks";
        size_t plaintext_len = sizeof(plaintext) - 1;
        ciphertext_message *message = 0;
        result = group_cipher_encrypt(alice_group_cipher, (uint8_t *)plaintext, plaintext_len, &message);
        ck_assert_int_eq(result, 0);
        inflight[i] = message;
    }

    signal_buffer *buffer = 0;

    /* Try decrypting in-flight message 1000 */
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)inflight[1000], 0, &buffer);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(buffer, 0);
    signal_buffer_free(buffer); buffer = 0;

    /* Try decrypting in-flight message 2009 */
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)inflight[2009], 0, &buffer);
    ck_assert_int_eq(result, 0);
    ck_assert_ptr_ne(buffer, 0);
    signal_buffer_free(buffer); buffer = 0;

    /* Try decrypting in-flight message 0, which should fail */
    result = group_cipher_decrypt(bob_group_cipher, (sender_key_message *)inflight[0], 0, &buffer);
    ck_assert_int_eq(result, SG_ERR_DUPLICATE_MESSAGE);
    signal_buffer_free(buffer); buffer = 0;

    /* Cleanup */
    for(i = 0; i < 2010; i++) {
        if(inflight[i]) {
            SIGNAL_UNREF(inflight[i]);
        }
    }
    SIGNAL_UNREF(alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_cipher_free(alice_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

START_TEST(test_invalid_signature_key)
{
    int result = 0;

    /* Create the test data stores */
    signal_protocol_store_context *alice_store = 0;
    setup_test_store_context(&alice_store, global_context);

    signal_protocol_store_context *bob_store = 0;
    setup_test_store_context(&bob_store, global_context);

    /* Create the session builders */
    group_session_builder *alice_session_builder = 0;
    result = group_session_builder_create(&alice_session_builder, alice_store, global_context);
    ck_assert_int_eq(result, 0);

    group_session_builder *bob_session_builder = 0;
    result = group_session_builder_create(&bob_session_builder, bob_store, global_context);
    ck_assert_int_eq(result, 0);

    /* Create the group cipher for Bob */
    group_cipher *bob_group_cipher = 0;
    result = group_cipher_create(&bob_group_cipher, bob_store, &GROUP_SENDER, global_context);

    /* Create a sender key distribution message from Alice to Bob */
    sender_key_distribution_message *sent_alice_distribution_message = 0;
    result = group_session_builder_create_session(alice_session_builder, &sent_alice_distribution_message, &GROUP_SENDER);
    ck_assert_int_eq(result, 0);

    sender_key_distribution_message *received_alice_distribution_message = 0;
    signal_buffer *serialized_distribution_message =
            ciphertext_message_get_serialized((ciphertext_message *)sent_alice_distribution_message);
    result = sender_key_distribution_message_deserialize(&received_alice_distribution_message,
            signal_buffer_data(serialized_distribution_message),
            signal_buffer_len(serialized_distribution_message),
            global_context);
    ck_assert_int_eq(result, 0);

    /* Processing Alice's distribution message */
    result = group_session_builder_process_session(bob_session_builder, &GROUP_SENDER, received_alice_distribution_message);
    ck_assert_int_eq(result, 0);

    /* Encrypt a test message from Bob */
    static const char bob_plaintext[] = "smert ze smert";
    size_t bob_plaintext_len = sizeof(bob_plaintext) - 1;
    ciphertext_message *ciphertext_from_bob = 0;
    result = group_cipher_encrypt(bob_group_cipher,
            (const uint8_t *)bob_plaintext, bob_plaintext_len,
            &ciphertext_from_bob);
    ck_assert_int_eq(result, SG_ERR_INVALID_KEY);

    /* Cleanup */
    SIGNAL_UNREF(ciphertext_from_bob);
    SIGNAL_UNREF(received_alice_distribution_message);
    SIGNAL_UNREF(sent_alice_distribution_message);
    group_cipher_free(bob_group_cipher);
    group_session_builder_free(bob_session_builder);
    group_session_builder_free(alice_session_builder);
    signal_protocol_store_context_destroy(bob_store);
    signal_protocol_store_context_destroy(alice_store);
}
END_TEST

Suite *group_cipher_suite(void)
{
    Suite *suite = suite_create("group_cipher");

    TCase *tcase = tcase_create("case");
    tcase_add_checked_fixture(tcase, test_setup, test_teardown);
    tcase_add_test(tcase, test_no_session);
    tcase_add_test(tcase, test_basic_encrypt_decrypt);
    tcase_add_test(tcase, test_basic_ratchet);
    tcase_add_test(tcase, test_late_join);
    tcase_add_test(tcase, test_out_of_order);
    tcase_add_test(tcase, test_encrypt_no_session);
    tcase_add_test(tcase, test_too_far_in_future);
    tcase_add_test(tcase, test_message_key_limit);
    tcase_add_test(tcase, test_invalid_signature_key);
    suite_add_tcase(suite, tcase);

    return suite;
}

int main(void)
{
    int number_failed;
    Suite *suite;
    SRunner *runner;

    suite = group_cipher_suite();
    runner = srunner_create(suite);

    srunner_run_all(runner, CK_VERBOSE);
    number_failed = srunner_ntests_failed(runner);
    srunner_free(runner);
    return (number_failed == 0) ? EXIT_SUCCESS : EXIT_FAILURE;
}
