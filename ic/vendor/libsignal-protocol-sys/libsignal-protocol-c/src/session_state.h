#ifndef SESSION_STATE_H
#define SESSION_STATE_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/*------------------------------------------------------------------------*/

int session_state_create(session_state **state, signal_context *global_context);
int session_state_serialize(signal_buffer **buffer, session_state *state);
int session_state_deserialize(session_state **state, const uint8_t *data, size_t len, signal_context *global_context);
int session_state_copy(session_state **state, session_state *other_state, signal_context *global_context);

void session_state_set_session_version(session_state *state, uint32_t version);
uint32_t session_state_get_session_version(const session_state *state);

void session_state_set_local_identity_key(session_state *state, ec_public_key *identity_key);
ec_public_key *session_state_get_local_identity_key(const session_state *state);

void session_state_set_remote_identity_key(session_state *state, ec_public_key *identity_key);
ec_public_key *session_state_get_remote_identity_key(const session_state *state);

void session_state_set_root_key(session_state *state, ratchet_root_key *root_key);
ratchet_root_key *session_state_get_root_key(const session_state *state);

void session_state_set_previous_counter(session_state *state, uint32_t counter);
uint32_t session_state_get_previous_counter(const session_state *state);

void session_state_set_sender_chain(session_state *state, ec_key_pair *sender_ratchet_key_pair, ratchet_chain_key *chain_key);
ec_public_key *session_state_get_sender_ratchet_key(const session_state *state);
ec_key_pair *session_state_get_sender_ratchet_key_pair(const session_state *state);
ratchet_chain_key *session_state_get_sender_chain_key(const session_state *state);
int session_state_set_sender_chain_key(session_state *state, ratchet_chain_key *chain_key);
int session_state_has_sender_chain(const session_state *state);

int session_state_has_message_keys(session_state *state, ec_public_key *sender_ephemeral, uint32_t counter);
int session_state_remove_message_keys(session_state *state,
        ratchet_message_keys *message_keys_result,
        ec_public_key *sender_ephemeral, uint32_t counter);
int session_state_set_message_keys(session_state *state,
        ec_public_key *sender_ephemeral, ratchet_message_keys *message_keys);

int session_state_add_receiver_chain(session_state *state, ec_public_key *sender_ratchet_key, ratchet_chain_key *chain_key);
int session_state_set_receiver_chain_key(session_state *state, ec_public_key *sender_ephemeral, ratchet_chain_key *chain_key);
ratchet_chain_key *session_state_get_receiver_chain_key(session_state *state, ec_public_key *sender_ephemeral);

void session_state_set_pending_key_exchange(session_state *state,
        uint32_t sequence,
        ec_key_pair *our_base_key, ec_key_pair *our_ratchet_key,
        ratchet_identity_key_pair *our_identity_key);
uint32_t session_state_get_pending_key_exchange_sequence(session_state *state);
ec_key_pair *session_state_get_pending_key_exchange_base_key(const session_state *state);
ec_key_pair *session_state_get_pending_key_exchange_ratchet_key(const session_state *state);
ratchet_identity_key_pair *session_state_get_pending_key_exchange_identity_key(const session_state *state);
int session_state_has_pending_key_exchange(const session_state *state);

void session_state_set_unacknowledged_pre_key_message(session_state *state,
        const uint32_t *pre_key_id, uint32_t signed_pre_key_id, ec_public_key *base_key);
int session_state_unacknowledged_pre_key_message_has_pre_key_id(const session_state *state);
uint32_t session_state_unacknowledged_pre_key_message_get_pre_key_id(const session_state *state);
uint32_t session_state_unacknowledged_pre_key_message_get_signed_pre_key_id(const session_state *state);
ec_public_key *session_state_unacknowledged_pre_key_message_get_base_key(const session_state *state);
int session_state_has_unacknowledged_pre_key_message(const session_state *state);
void session_state_clear_unacknowledged_pre_key_message(session_state *state);

void session_state_set_remote_registration_id(session_state *state, uint32_t id);
uint32_t session_state_get_remote_registration_id(const session_state *state);

void session_state_set_local_registration_id(session_state *state, uint32_t id);
uint32_t session_state_get_local_registration_id(const session_state *state);

void session_state_set_needs_refresh(session_state *state, int value);
int session_state_get_needs_refresh(const session_state *state);

void session_state_set_alice_base_key(session_state *state, ec_public_key *key);
ec_public_key *session_state_get_alice_base_key(const session_state *state);

void session_state_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* SESSION_STATE_H */
