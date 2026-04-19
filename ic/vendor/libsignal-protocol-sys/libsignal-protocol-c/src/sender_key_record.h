#ifndef SENDER_KEY_RECORD_H
#define SENDER_KEY_RECORD_H

#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int sender_key_record_create(sender_key_record **record,
        signal_context *global_context);
int sender_key_record_serialize(signal_buffer **buffer, sender_key_record *record);
int sender_key_record_deserialize(sender_key_record **record, const uint8_t *data, size_t len, signal_context *global_context);
int sender_key_record_copy(sender_key_record **record, sender_key_record *other_state, signal_context *global_context);

int sender_key_record_is_empty(sender_key_record *record);
int sender_key_record_get_sender_key_state(sender_key_record *record, sender_key_state **state);
int sender_key_record_get_sender_key_state_by_id(sender_key_record *record, sender_key_state **state, uint32_t key_id);
int sender_key_record_add_sender_key_state(sender_key_record *record,
        uint32_t id, uint32_t iteration, signal_buffer *chain_key, ec_public_key *signature_key);
int sender_key_record_set_sender_key_state(sender_key_record *record,
        uint32_t id, uint32_t iteration, signal_buffer *chain_key, ec_key_pair *signature_key_pair);

signal_buffer *sender_key_record_get_user_record(const sender_key_record *record);
void sender_key_record_set_user_record(sender_key_record *record, signal_buffer *user_record);

void sender_key_record_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* SENDER_KEY_RECORD_H */
