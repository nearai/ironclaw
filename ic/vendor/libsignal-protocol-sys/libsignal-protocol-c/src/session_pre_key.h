#ifndef SESSION_PRE_KEY_H
#define SESSION_PRE_KEY_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

#define PRE_KEY_MEDIUM_MAX_VALUE 0xFFFFFF

/*------------------------------------------------------------------------*/

int session_pre_key_create(session_pre_key **pre_key, uint32_t id, ec_key_pair *key_pair);
int session_pre_key_serialize(signal_buffer **buffer, const session_pre_key *pre_key);
int session_pre_key_deserialize(session_pre_key **pre_key, const uint8_t *data, size_t len, signal_context *global_context);

uint32_t session_pre_key_get_id(const session_pre_key *pre_key);
ec_key_pair *session_pre_key_get_key_pair(const session_pre_key *pre_key);

void session_pre_key_destroy(signal_type_base *type);

/*------------------------------------------------------------------------*/

int session_signed_pre_key_create(session_signed_pre_key **pre_key,
        uint32_t id, uint64_t timestamp, ec_key_pair *key_pair,
        const uint8_t *signature, size_t signature_len);
int session_signed_pre_key_serialize(signal_buffer **buffer, const session_signed_pre_key *pre_key);
int session_signed_pre_key_deserialize(session_signed_pre_key **pre_key, const uint8_t *data, size_t len, signal_context *global_context);

uint32_t session_signed_pre_key_get_id(const session_signed_pre_key *pre_key);
uint64_t session_signed_pre_key_get_timestamp(const session_signed_pre_key *pre_key);
ec_key_pair *session_signed_pre_key_get_key_pair(const session_signed_pre_key *pre_key);
const uint8_t *session_signed_pre_key_get_signature(const session_signed_pre_key *pre_key);
size_t session_signed_pre_key_get_signature_len(const session_signed_pre_key *pre_key);

void session_signed_pre_key_destroy(signal_type_base *type);

/*------------------------------------------------------------------------*/

int session_pre_key_bundle_create(session_pre_key_bundle **bundle,
        uint32_t registration_id, int device_id, uint32_t pre_key_id,
        ec_public_key *pre_key_public,
        uint32_t signed_pre_key_id, ec_public_key *signed_pre_key_public,
        const uint8_t *signed_pre_key_signature_data, size_t signed_pre_key_signature_len,
        ec_public_key *identity_key);

uint32_t session_pre_key_bundle_get_registration_id(const session_pre_key_bundle *bundle);
int session_pre_key_bundle_get_device_id(const session_pre_key_bundle *bundle);
uint32_t session_pre_key_bundle_get_pre_key_id(const session_pre_key_bundle *bundle);
ec_public_key *session_pre_key_bundle_get_pre_key(const session_pre_key_bundle *bundle);
uint32_t session_pre_key_bundle_get_signed_pre_key_id(const session_pre_key_bundle *bundle);
ec_public_key *session_pre_key_bundle_get_signed_pre_key(const session_pre_key_bundle *bundle);
signal_buffer *session_pre_key_bundle_get_signed_pre_key_signature(const session_pre_key_bundle *bundle);
ec_public_key *session_pre_key_bundle_get_identity_key(const session_pre_key_bundle *bundle);

void session_pre_key_bundle_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* SESSION_PRE_KEY_H */
