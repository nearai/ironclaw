#ifndef GROUP_SESSION_BUILDER_H
#define GROUP_SESSION_BUILDER_H

#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Group session builder is responsible for setting up group sender key encrypted sessions.
 *
 * Once a session has been established, group_cipher can be used to
 * encrypt/decrypt messages in that session.
 * <p>
 * The built sessions are unidirectional: they can be used either for sending
 * or for receiving, but not both.
 *
 * Sessions are constructed per (groupId + senderId + deviceId) tuple.  Remote logical users
 * are identified by their senderId, and each logical recipientId can have multiple physical
 * devices.
 */

/**
 * Constructs a group session builder.
 *
 * The store and global contexts must remain valid for the lifetime of the
 * session builder.
 *
 * When finished, free the returned instance by calling group_session_builder_free().
 *
 * @param builder set to a freshly allocated group session builder instance
 * @param store the signal_protocol_store_context to store all state information in
 * @param global_context the global library context
 * @return 0 on success, or negative on failure
 */
int group_session_builder_create(group_session_builder **builder,
        signal_protocol_store_context *store, signal_context *global_context);

/**
 * Construct a group session for receiving messages from senderKeyName.
 *
 * @param sender_key_name the (groupId, senderId, deviceId) tuple associated
 *     with the sender_key_distribution_message
 * @param distribution_message a received sender_key_distribution_message
 * @return 0 on success, or negative on failure
 */
int group_session_builder_process_session(group_session_builder *builder,
        const signal_protocol_sender_key_name *sender_key_name,
        sender_key_distribution_message *distribution_message);

/**
 * Construct a group session for sending messages.
 *
 * @param distribution_message a distribution message to be allocated and populated
 * @param sender_key_name the (groupId, senderId, deviceId) tuple. In this
 *     case, the sender should be the caller
 * @return 0 on success, or negative on failure
 */
int group_session_builder_create_session(group_session_builder *builder,
        sender_key_distribution_message **distribution_message,
        const signal_protocol_sender_key_name *sender_key_name);

void group_session_builder_free(group_session_builder *builder);

#ifdef __cplusplus
}
#endif

#endif /* GROUP_SESSION_BUILDER_H */
