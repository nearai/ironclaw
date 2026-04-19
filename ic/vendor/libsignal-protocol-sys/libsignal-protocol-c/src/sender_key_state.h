#ifndef SENDER_KEY_STATE_H
#define SENDER_KEY_STATE_H

#include <stdint.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int sender_key_state_create(sender_key_state **state,
        uint32_t id, sender_chain_key *chain_key,
        ec_public_key *signature_public_key, ec_private_key *signature_private_key,
        signal_context *global_context);
int sender_key_state_serialize(signal_buffer **buffer, sender_key_state *state);
int sender_key_state_deserialize(sender_key_state **state, const uint8_t *data, size_t len, signal_context *global_context);
int sender_key_state_copy(sender_key_state **state, sender_key_state *other_state, signal_context *global_context);

uint32_t sender_key_state_get_key_id(sender_key_state *state);
sender_chain_key *sender_key_state_get_chain_key(sender_key_state *state);
void sender_key_state_set_chain_key(sender_key_state *state, sender_chain_key *chain_key);
ec_public_key *sender_key_state_get_signing_key_public(sender_key_state *state);
ec_private_key *sender_key_state_get_signing_key_private(sender_key_state *state);
int sender_key_state_has_sender_message_key(sender_key_state *state, uint32_t iteration);
int sender_key_state_add_sender_message_key(sender_key_state *state, sender_message_key *message_key);
sender_message_key *sender_key_state_remove_sender_message_key(sender_key_state *state, uint32_t iteration);

void sender_key_state_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* SENDER_KEY_STATE_H */
