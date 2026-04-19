#ifndef SESSION_CIPHER_H
#define SESSION_CIPHER_H

#include <stdint.h>
#include <stddef.h>
#include "signal_protocol_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * The main entry point for Signal Protocol encrypt/decrypt operations.
 *
 * Once a session has been established with session_builder,
 * this class can be used for all encrypt/decrypt operations within
 * that session.
 */

/**
 * Construct a session cipher for encrypt/decrypt operations on a session.
 * In order to use session_cipher, a session must have already been created
 * and stored using session_builder.
 *
 * The store and global contexts must remain valid for the lifetime of the
 * session cipher.
 *
 * When finished, free the returned instance by calling session_cipher_free().
 *
 * @param cipher set to a freshly allocated session cipher instance
 * @param store the signal_protocol_store_context to store all state information in
 * @param remote_address the remote address that messages will be encrypted to or decrypted from.
 * @param global_context the global library context
 * @return 0 on success, or negative on failure
 */
int session_cipher_create(session_cipher **cipher,
        signal_protocol_store_context *store, const signal_protocol_address *remote_address,
        signal_context *global_context);

/**
 * Set the optional user data pointer for the session cipher.
 *
 * This is to give callback functions a way of accessing app specific
 * context information for this cipher.
 */
void session_cipher_set_user_data(session_cipher *cipher, void *user_data);

/**
 * Get the optional user data pointer for the session cipher.
 *
 * This is to give callback functions a way of accessing app specific
 * context information for this cipher.
 */
void *session_cipher_get_user_data(session_cipher *cipher);

/**
 * Set the callback function that is called during the decrypt process.
 *
 * The callback function is called from within
 * session_cipher_decrypt_pre_key_signal_message() and
 * session_cipher_decrypt_signal_message() after decryption is complete
 * but before the updated session state has been committed to the session
 * store. If the callback function returns a negative value, then the
 * decrypt function will immediately fail with an error.
 *
 * This a callback allows some implementations to store the committed plaintext
 * to their local message store first, in case they are concerned with a crash
 * or write error happening between the time the session state is updated but
 * before they're able to successfully store the plaintext to disk.
 *
 * @param callback the callback function to set
 */
void session_cipher_set_decryption_callback(session_cipher *cipher,
        int (*callback)(session_cipher *cipher, signal_buffer *plaintext, void *decrypt_context));

/**
 * Encrypt a message.
 *
 * @param padded_message The plaintext message bytes, optionally padded to a constant multiple.
 * @param padded_message_len The length of the data pointed to by padded_message
 * @param encrypted_message Set to a ciphertext message encrypted to the recipient+device tuple.
 *
 * @return SG_SUCCESS on success, negative on error
 */
int session_cipher_encrypt(session_cipher *cipher,
        const uint8_t *padded_message, size_t padded_message_len,
        ciphertext_message **encrypted_message);

/**
 * Decrypt a message.
 *
 * @param ciphertext The pre_key_signal_message to decrypt.
 * @param decrypt_context Optional context pointer associated with the
 *   ciphertext, which is passed to the decryption callback function
 * @param plaintext Set to a newly allocated buffer containing the plaintext.
 *
 * @retval SG_SUCCESS Success
 * @retval SG_ERR_INVALID_MESSAGE if the input is not valid ciphertext.
 * @retval SG_ERR_DUPLICATE_MESSAGE if the input is a message that has already been received.
 * @retval SG_ERR_LEGACY_MESSAGE if the input is a message formatted by a protocol version that
 *                               is no longer supported.
 * @retval SG_ERR_INVALID_KEY_ID when there is no local pre_key_record
 *                               that corresponds to the pre key ID in the message.
 * @retval SG_ERR_INVALID_KEY when the message is formatted incorrectly.
 * @retval SG_ERR_UNTRUSTED_IDENTITY when the identity key of the sender is untrusted.
 */
int session_cipher_decrypt_pre_key_signal_message(session_cipher *cipher,
        pre_key_signal_message *ciphertext, void *decrypt_context,
        signal_buffer **plaintext);

/**
 * Decrypt a message.
 *
 * @param ciphertext The signal_message to decrypt.
 * @param decrypt_context Optional context pointer associated with the
 *   ciphertext, which is passed to the decryption callback function
 * @param plaintext Set to a newly allocated buffer containing the plaintext.
 *
 * @retval SG_SUCCESS Success
 * @retval SG_ERR_INVALID_MESSAGE if the input is not valid ciphertext.
 * @retval SG_ERR_DUPLICATE_MESSAGE if the input is a message that has already been received.
 * @retval SG_ERR_LEGACY_MESSAGE if the input is a message formatted by a protocol version that
 *                               is no longer supported.
 * @retval SG_ERR_NO_SESSION if there is no established session for this contact.
 */
int session_cipher_decrypt_signal_message(session_cipher *cipher,
        signal_message *ciphertext, void *decrypt_context,
        signal_buffer **plaintext);

/**
 * Gets the remote registration ID for this session cipher.
 *
 * @param remote_id Set to the value of the remote registration ID
 *
 * @return SG_SUCCESS on success, negative on error
 */
int session_cipher_get_remote_registration_id(session_cipher *cipher, uint32_t *remote_id);

/**
 * Gets the version of the session associated with this session cipher.
 *
 * @param version Set to the value of the session version
 *
 * @retval SG_SUCCESS Success
 * @retval SG_ERR_NO_SESSION if no session could be found
 */
int session_cipher_get_session_version(session_cipher *cipher, uint32_t *version);

void session_cipher_free(session_cipher *cipher);

#ifdef __cplusplus
}
#endif

#endif /* SESSION_CIPHER_H */
