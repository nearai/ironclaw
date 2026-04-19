#ifndef HKDF_H
#define HKDF_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

int hkdf_create(hkdf_context **context, int message_version, signal_context *global_context);

ssize_t hkdf_derive_secrets(hkdf_context *context,
        uint8_t **output,
        const uint8_t *input_key_material, size_t input_key_material_len,
        const uint8_t *salt, size_t salt_len,
        const uint8_t *info, size_t info_len,
        size_t output_len);

int hkdf_compare(const hkdf_context *context1, const hkdf_context *context2);

void hkdf_destroy(signal_type_base *type);

#ifdef __cplusplus
}
#endif

#endif /* HKDF_H */
