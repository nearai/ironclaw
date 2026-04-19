#ifndef SESSION_RECORD_H
#define SESSION_RECORD_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int session_record_create(session_record **record, session_state *state, signal_context *global_context);
int session_record_serialize(signal_buffer **buffer, const session_record *record);
int session_record_deserialize(session_record **record, const uint8_t *data, size_t len, signal_context *global_context);
int session_record_copy(session_record **record, session_record *other_record, signal_context *global_context);

int session_record_has_session_state(session_record *record, uint32_t version, const ec_public_key *alice_base_key);
session_state *session_record_get_state(session_record *record);
void session_record_set_state(session_record *record, session_state *state);

session_record_state_node *session_record_get_previous_states_head(const session_record *record);
session_state *session_record_get_previous_states_element(const session_record_state_node *node);
session_record_state_node *session_record_get_previous_states_next(const session_record_state_node *node);

/**
 * Removes the specified node in the previous states list.
 * @param node the node to remove
 * @return the node immediately following the removed node, or null if at the end of the list
 */
session_record_state_node *session_record_get_previous_states_remove(session_record *record, session_record_state_node *node);

int session_record_is_fresh(session_record *record);

/**
 * Move the current session_state into the list of "previous" session states,
 * and replace the current session_state with a fresh reset instance.
 *
 * @return 0 on success, negative on failure
 */
int session_record_archive_current_state(session_record *record);

int session_record_promote_state(session_record *record, session_state *promoted_state);

signal_buffer *session_record_get_user_record(const session_record *record);
void session_record_set_user_record(session_record *record, signal_buffer *user_record);

void session_record_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* SESSION_RECORD_H */
