#ifndef DEVICE_CONSISTENCY_H
#define DEVICE_CONSISTENCY_H

#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int device_consistency_signature_create(device_consistency_signature **signature,
        const uint8_t *signature_data, size_t signature_len,
        const uint8_t *vrf_output_data, size_t vrf_output_len);

signal_buffer *device_consistency_signature_get_signature(const device_consistency_signature *signature);
signal_buffer *device_consistency_signature_get_vrf_output(const device_consistency_signature *signature);

void device_consistency_signature_destroy(signal_type_base *type);

int device_consistency_commitment_create(device_consistency_commitment **commitment,
        uint32_t generation, ec_public_key_list *identity_key_list,
        signal_context *global_context);

uint32_t device_consistency_commitment_get_generation(const device_consistency_commitment *commitment);
signal_buffer *device_consistency_commitment_get_serialized(const device_consistency_commitment *commitment);

void device_consistency_commitment_destroy(signal_type_base *type);

int device_consistency_message_create_from_pair(device_consistency_message **message,
        device_consistency_commitment *commitment,
        ec_key_pair *identity_key_pair,
        signal_context *global_context);
int device_consistency_message_create_from_serialized(device_consistency_message **message,
        device_consistency_commitment *commitment,
        const uint8_t *serialized_data, size_t serialized_len,
        ec_public_key *identity_key,
        signal_context *global_context);

signal_buffer *device_consistency_message_get_serialized(const device_consistency_message *message);
device_consistency_signature *device_consistency_message_get_signature(const device_consistency_message *message);
uint32_t device_consistency_signature_get_generation(const device_consistency_message *message);

void device_consistency_message_destroy(signal_type_base *type);

int device_consistency_code_generate_for(device_consistency_commitment *commitment,
        device_consistency_signature_list *signatures,
        char **code_string,
        signal_context *global_context);

device_consistency_signature_list *device_consistency_signature_list_alloc(void);
device_consistency_signature_list *device_consistency_signature_list_copy(const device_consistency_signature_list *list);
int device_consistency_signature_list_push_back(device_consistency_signature_list *list, device_consistency_signature *value);
unsigned int device_consistency_signature_list_size(const device_consistency_signature_list *list);
device_consistency_signature *device_consistency_signature_list_at(const device_consistency_signature_list *list, unsigned int index);
void device_consistency_signature_list_free(device_consistency_signature_list *list);

#ifdef __cplusplus
}
#endif

#endif /* DEVICE_CONSISTENCY_H */
