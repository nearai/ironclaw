#ifndef SESSION_BUILDER_INTERNAL_H
#define SESSION_BUILDER_INTERNAL_H

#include <stdint.h>
#include "signal_protocol_types.h"

/**
 * Build a new session from a received pre_key_signal_message.
 *
 * After a session is constructed in this way, the embedded signal_message
 * can be decrypted.
 *
 * @param message The received pre_key_signal_message.
 * @param unsigned_pre_key_id set to the unsigned pre key ID, if available.
 *     Return value indicates whether or not this value is available.
 * @retval 0 Success, no unsigned pre key value available
 * @retval 1 Success, an unsigned pre key is available
 * @retval SG_ERR_INVALID_KEY_ID when there is no local pre_key_record that
 *                               corresponds to the PreKey ID in the message.
 * @retval SG_ERR_INVALID_KEY when the message is formatted incorrectly.
 * @retval SG_ERR_UNTRUSTED_IDENTITY when the identity key of the sender is untrusted.
 */
int session_builder_process_pre_key_signal_message(session_builder *builder,
        session_record *record, pre_key_signal_message *message, uint32_t *unsigned_pre_key_id);

#endif /* SESSION_BUILDER_INTERNAL_H */
