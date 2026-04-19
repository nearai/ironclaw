#ifndef FINGERPRINT_H
#define FINGERPRINT_H

#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Construct a fingerprint generator for 60 digit numerics.
 *
 * @param generator set to a freshly allocated generator instance
 * @param iterations The number of internal iterations to perform in the process of
 *                   generating a fingerprint. This needs to be constant, and synchronized
 *                   across all clients.
 *
 *                   The higher the iteration count, the higher the security level:
 *                   - 1024 ~ 109.7 bits
 *                   - 1400 > 110 bits
 *                   - 5200 > 112 bits
 * @param scannable_version The format version for the scannable fingerprint (0 or 1)
 * @param global_context the global library context
 * @return 0 on success, or negative on failure
 */
int fingerprint_generator_create(fingerprint_generator **generator,
        int iterations, int scannable_version,
        signal_context *global_context);

/**
 * Generate a scannable and displayble fingerprint.
 *
 * @param local_stable_identifier The client's "stable" identifier.
 * @param local_identity_key The client's identity key.
 * @param remote_stable_identifier The remote party's "stable" identifier.
 * @param remote_identity_key The remote party's identity key.
 * @param fingerprint_val Set to a freshly allocated unique fingerprint for this conversation
 * @return 0 on success, or negative on failure
 */
int fingerprint_generator_create_for(fingerprint_generator *generator,
        const char *local_stable_identifier, const ec_public_key *local_identity_key,
        const char *remote_stable_identifier, const ec_public_key *remote_identity_key,
        fingerprint **fingerprint_val);

/**
 * Generate a scannable and displayble fingerprint for a list of keys
 *
 * @param local_stable_identifier The client's "stable" identifier.
 * @param local_identity_key_list The client's identity key list.
 * @param remote_stable_identifier The remote party's "stable" identifier.
 * @param remote_identity_key_list The remote party's identity key list.
 * @param fingerprint_val Set to a freshly allocated unique fingerprint for this conversation
 * @return 0 on success, or negative on failure
 */
int fingerprint_generator_create_for_list(fingerprint_generator *generator,
        const char *local_stable_identifier, const ec_public_key_list *local_identity_key_list,
        const char *remote_stable_identifier, const ec_public_key_list *remote_identity_key_list,
        fingerprint **fingerprint_val);

void fingerprint_generator_free(fingerprint_generator *generator);

int fingerprint_create(fingerprint **fingerprint_val, displayable_fingerprint *displayable, scannable_fingerprint *scannable);
displayable_fingerprint *fingerprint_get_displayable(const fingerprint *fingerprint_val);
scannable_fingerprint *fingerprint_get_scannable(const fingerprint *fingerprint_val);
void fingerprint_destroy(signal_type_base *type);

int displayable_fingerprint_create(displayable_fingerprint **displayable, const char *local_fingerprint, const char *remote_fingerprint);
const char *displayable_fingerprint_local(const displayable_fingerprint *displayable);
const char *displayable_fingerprint_remote(const displayable_fingerprint *displayable);
const char *displayable_fingerprint_text(const displayable_fingerprint *displayable);
void displayable_fingerprint_destroy(signal_type_base *type);

int scannable_fingerprint_create(scannable_fingerprint **scannable,
        uint32_t version,
        const char *local_stable_identifier, const signal_buffer *local_fingerprint,
        const char *remote_stable_identifier, const signal_buffer *remote_fingerprint);

int scannable_fingerprint_serialize(signal_buffer **buffer, const scannable_fingerprint *scannable);
int scannable_fingerprint_deserialize(scannable_fingerprint **scannable, const uint8_t *data, size_t len, signal_context *global_context);
uint32_t scannable_fingerprint_get_version(const scannable_fingerprint *scannable);
const char *scannable_fingerprint_get_local_stable_identifier(const scannable_fingerprint *scannable);
signal_buffer *scannable_fingerprint_get_local_fingerprint(const scannable_fingerprint *scannable);
const char *scannable_fingerprint_get_remote_stable_identifier(const scannable_fingerprint *scannable);
signal_buffer *scannable_fingerprint_get_remote_fingerprint(const scannable_fingerprint *scannable);

/**
 * Compare a scanned QR code with what we expect.
 * @param scannable The local scannable data
 * @param other_scannable The data from the scanned code
 * @retval 1 if the scannable codes match
 * @retval 0 if the scannable codes do not match
 * @retval SG_ERR_FP_VERSION_MISMATCH if the scanned fingerprint is the wrong version
 * @retval SG_ERR_FP_IDENT_MISMATCH if the scanned fingerprint is for the wrong stable identifier
 */
int scannable_fingerprint_compare(const scannable_fingerprint *scannable, const scannable_fingerprint *other_scannable);

void scannable_fingerprint_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* FINGERPRINT_H */
