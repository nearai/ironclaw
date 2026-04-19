#ifndef CURVE_H
#define CURVE_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

#define CURVE_SIGNATURE_LEN 64
#define VRF_SIGNATURE_LEN 96

int curve_internal_fast_tests(int silent);

int curve_decode_point(ec_public_key **public_key, const uint8_t *key_data, size_t key_len, signal_context *global_context);
int ec_public_key_compare(const ec_public_key *key1, const ec_public_key *key2);
int ec_public_key_memcmp(const ec_public_key *key1, const ec_public_key *key2);

/**
 * Serialize the public key into a buffer that can be stored.
 * The format of this data is compatible with the input format of
 * curve_decode_point().
 *
 * @param buffer Pointer to a buffer that will be allocated by this function
 *     and filled with the contents of the key. The caller is responsible for
 *     freeing this buffer with signal_buffer_free().
 * @param key Key to serialize
 * @return 0 on success, negative on failure
 */
int ec_public_key_serialize(signal_buffer **buffer, const ec_public_key *key);

void ec_public_key_destroy(signal_type_base *type);

int curve_decode_private_point(ec_private_key **private_key, const uint8_t *key_data, size_t key_len, signal_context *global_context);
int ec_private_key_compare(const ec_private_key *key1, const ec_private_key *key2);

/**
 * Serialize the private key into a buffer that can be stored.
 * The format of this data is compatible with the input format of
 * curve_decode_private_point().
 *
 * @param buffer Pointer to a buffer that will be allocated by this function
 *     and filled with the contents of the key. The caller is responsible for
 *     freeing this buffer with signal_buffer_free().
 * @param key Key to serialize
 * @return 0 on success, negative on failure
 */
int ec_private_key_serialize(signal_buffer **buffer, const ec_private_key *key);

void ec_private_key_destroy(signal_type_base *type);

int ec_key_pair_create(ec_key_pair **key_pair, ec_public_key *public_key, ec_private_key *private_key);
ec_public_key *ec_key_pair_get_public(const ec_key_pair *key_pair);
ec_private_key *ec_key_pair_get_private(const ec_key_pair *key_pair);
void ec_key_pair_destroy(signal_type_base *type);

int curve_generate_private_key(signal_context *context, ec_private_key **private_key);
int curve_generate_public_key(ec_public_key **public_key, const ec_private_key *private_key);

/**
 * Generates a Curve25519 keypair.
 *
 * @param key_pair Set to a randomly generated Curve25519 keypair on success.
 * @return 0 on success, negative on failure
 */
int curve_generate_key_pair(signal_context *context, ec_key_pair **key_pair);

/**
 * Allocate a new ec_public_key list
 *
 * @return pointer to the allocated list, or 0 on failure
 */
ec_public_key_list *ec_public_key_list_alloc(void);

/**
 * Copy an ec_public_key list
 *
 * @return pointer to the copy of the list, or 0 on failure
 */
ec_public_key_list *ec_public_key_list_copy(const ec_public_key_list *list);

/**
 * Push a new value onto the end of the list
 *
 * @param list the list
 * @param value the value to push
 * @return 0 on success, negative on failure
 */
int ec_public_key_list_push_back(ec_public_key_list *list, ec_public_key *value);

/**
 * Gets the size of the list.
 *
 * @param list the list
 * @return the size of the list
 */
unsigned int ec_public_key_list_size(const ec_public_key_list *list);

/**
 * Gets the value of the element at a particular index in the list
 *
 * @param list the list
 * @param index the index within the list
 * @return the value
 */
ec_public_key *ec_public_key_list_at(const ec_public_key_list *list, unsigned int index);

/**
 * Sorts the list based on a comparison of the key data.
 *
 * @param list the list
 */
void ec_public_key_list_sort(ec_public_key_list *list);

/**
 * Free the ec_public_key list
 * @param list the list to free
 */
void ec_public_key_list_free(ec_public_key_list *list);

/**
 * Calculates an ECDH agreement.
 *
 * @param shared_key_data Set to a 32-byte shared secret on success.
 * @param public_key The Curve25519 (typically remote party's) public key.
 * @param private_key The Curve25519 (typically yours) private key.
 * @return length of the shared secret on success, negative on failure
 */
int curve_calculate_agreement(uint8_t **shared_key_data, const ec_public_key *public_key, const ec_private_key *private_key);

/**
 * Verify a Curve25519 signature.
 *
 * @param signing_key The Curve25519 public key the signature belongs to.
 * @param message_data The message that was signed.
 * @param message_len The length of the message that was signed.
 * @param signature_data The signature to verify.
 * @param signature_len The length of the signature to verify.
 * @return 1 if valid, 0 if invalid, negative on failure
 */
int curve_verify_signature(const ec_public_key *signing_key,
        const uint8_t *message_data, size_t message_len,
        const uint8_t *signature_data, size_t signature_len);

/**
 * Calculates a Curve25519 signature.
 *
 * @param signature Set to a 64-byte signature on success.
 * @param signing_key The private Curve25519 key to create the signature with.
 * @param message_data The message to sign.
 * @param message_len The length of the message to sign.
 * @return 0 on success, negative on failure
 */
int curve_calculate_signature(signal_context *context,
        signal_buffer **signature,
        const ec_private_key *signing_key,
        const uint8_t *message_data, size_t message_len);

/**
 * Verify a Unique Curve25519 signature.
 *
 * @param vrf_output Set to VRF output on success
 * @param signing_key The Curve25519 public key the unique signature belongs to.
 * @param message_data The message that was signed.
 * @param message_len The length of the message that was signed.
 * @param signature_data The signature to verify.
 * @param signature_len The length of the signature to verify.
 * @return 1 if valid, 0 if invalid, negative on failure
 */
int curve_verify_vrf_signature(signal_context *context,
        signal_buffer **vrf_output,
        const ec_public_key *signing_key,
        const uint8_t *message_data, size_t message_len,
        const uint8_t *signature_data, size_t signature_len);

/**
 * Calculates a Unique Curve25519 signature.
 *
 * @param signature Set to a 96-byte signature on success.
 * @param signing_key The private Curve25519 key to create the signature with.
 * @param message_data The message to sign.
 * @param message_len The length of the message to sign.
 * @return 0 on success, negative on failure
 */
int curve_calculate_vrf_signature(signal_context *context,
        signal_buffer **signature,
        const ec_private_key *signing_key,
        const uint8_t *message_data, size_t message_len);

#ifdef __cplusplus
}
#endif

#endif /* CURVE_H */
