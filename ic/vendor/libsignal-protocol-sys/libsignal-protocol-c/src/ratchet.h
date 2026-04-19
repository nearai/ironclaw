#ifndef RATCHET_H
#define RATCHET_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int ratchet_chain_key_create(ratchet_chain_key **chain_key, hkdf_context *kdf,
        const uint8_t *key, size_t key_len, uint32_t index,
        signal_context *global_context);
int ratchet_chain_key_get_key(const ratchet_chain_key *chain_key, signal_buffer **buffer);
uint32_t ratchet_chain_key_get_index(const ratchet_chain_key *chain_key);
int ratchet_chain_key_get_message_keys(ratchet_chain_key *chain_key, ratchet_message_keys *message_keys);
int ratchet_chain_key_create_next(const ratchet_chain_key *chain_key, ratchet_chain_key **next_chain_key);
void ratchet_chain_key_destroy(signal_type_base *type);

int ratchet_root_key_create(ratchet_root_key **root_key, hkdf_context *kdf,
        const uint8_t *key, size_t key_len,
        signal_context *global_context);
int ratchet_root_key_create_chain(ratchet_root_key *root_key,
        ratchet_root_key **new_root_key, ratchet_chain_key **new_chain_key,
        ec_public_key *their_ratchet_key,
        ec_private_key *our_ratchet_key_private);
int ratchet_root_key_get_key(ratchet_root_key *root_key, signal_buffer **buffer);
int ratchet_root_key_compare(const ratchet_root_key *key1, const ratchet_root_key *key2);
void ratchet_root_key_destroy(signal_type_base *type);

int ratchet_identity_key_pair_create(
        ratchet_identity_key_pair **key_pair,
        ec_public_key *public_key,
        ec_private_key *private_key);
int ratchet_identity_key_pair_serialize(signal_buffer **buffer, const ratchet_identity_key_pair *key_pair);
int ratchet_identity_key_pair_deserialize(ratchet_identity_key_pair **key_pair, const uint8_t *data, size_t len, signal_context *global_context);
ec_public_key *ratchet_identity_key_pair_get_public(const ratchet_identity_key_pair *key_pair);
ec_private_key *ratchet_identity_key_pair_get_private(const ratchet_identity_key_pair *key_pair);
void ratchet_identity_key_pair_destroy(signal_type_base *type);

typedef struct symmetric_signal_protocol_parameters symmetric_signal_protocol_parameters;
typedef struct alice_signal_protocol_parameters alice_signal_protocol_parameters;
typedef struct bob_signal_protocol_parameters bob_signal_protocol_parameters;

int symmetric_signal_protocol_parameters_create(
        symmetric_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_base_key,
        ec_key_pair *our_ratchet_key,
        ec_public_key *their_base_key,
        ec_public_key *their_ratchet_key,
        ec_public_key *their_identity_key);
ratchet_identity_key_pair *symmetric_signal_protocol_parameters_get_our_identity_key(const symmetric_signal_protocol_parameters *parameters);
ec_key_pair *symmetric_signal_protocol_parameters_get_our_base_key(const symmetric_signal_protocol_parameters *parameters);
ec_key_pair *symmetric_signal_protocol_parameters_get_our_ratchet_key(const symmetric_signal_protocol_parameters *parameters);
ec_public_key *symmetric_signal_protocol_parameters_get_their_base_key(const symmetric_signal_protocol_parameters *parameters);
ec_public_key *symmetric_signal_protocol_parameters_get_their_ratchet_key(const symmetric_signal_protocol_parameters *parameters);
ec_public_key *symmetric_signal_protocol_parameters_get_their_identity_key(const symmetric_signal_protocol_parameters *parameters);
void symmetric_signal_protocol_parameters_destroy(signal_type_base *type);

int alice_signal_protocol_parameters_create(
        alice_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_base_key,
        ec_public_key *their_identity_key,
        ec_public_key *their_signed_pre_key,
        ec_public_key *their_one_time_pre_key,
        ec_public_key *their_ratchet_key);
void alice_signal_protocol_parameters_destroy(signal_type_base *type);

int bob_signal_protocol_parameters_create(
        bob_signal_protocol_parameters **parameters,
        ratchet_identity_key_pair *our_identity_key,
        ec_key_pair *our_signed_pre_key,
        ec_key_pair *our_one_time_pre_key,
        ec_key_pair *our_ratchet_key,
        ec_public_key *their_identity_key,
        ec_public_key *their_base_key);
void bob_signal_protocol_parameters_destroy(signal_type_base *type);

int ratcheting_session_symmetric_initialize(session_state *state, symmetric_signal_protocol_parameters *parameters, signal_context *global_context);
int ratcheting_session_alice_initialize(session_state *state, alice_signal_protocol_parameters *parameters, signal_context *global_context);
int ratcheting_session_bob_initialize(session_state *state, bob_signal_protocol_parameters *parameters, signal_context *global_context);

#ifdef __cplusplus
}
#endif

#endif /* RATCHET_H */
