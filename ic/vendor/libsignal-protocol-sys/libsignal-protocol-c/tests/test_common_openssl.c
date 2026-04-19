#include "test_common.h"

#include <limits.h>
#include <openssl/opensslv.h>
#include <openssl/evp.h>
#include <openssl/hmac.h>
#include <openssl/rand.h>
#include <openssl/sha.h>

int test_random_generator(uint8_t *data, size_t len, void *user_data)
{
    if(RAND_bytes(data, len)) {
        return 0;
    }
    else {
        return SG_ERR_UNKNOWN;
    }
}

int test_hmac_sha256_init(void **hmac_context, const uint8_t *key, size_t key_len, void *user_data)
{
#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
    HMAC_CTX *ctx = HMAC_CTX_new();
    if(!ctx) {
        return SG_ERR_NOMEM;
    }
#else
    HMAC_CTX *ctx = malloc(sizeof(HMAC_CTX));
    if(!ctx) {
        return SG_ERR_NOMEM;
    }
    HMAC_CTX_init(ctx);
#endif

    *hmac_context = ctx;

    if(HMAC_Init_ex(ctx, key, key_len, EVP_sha256(), 0) != 1) {
        return SG_ERR_UNKNOWN;
    }

    return 0;
}

int test_hmac_sha256_update(void *hmac_context, const uint8_t *data, size_t data_len, void *user_data)
{
    HMAC_CTX *ctx = hmac_context;
    int result = HMAC_Update(ctx, data, data_len);
    return (result == 1) ? 0 : -1;
}

int test_hmac_sha256_final(void *hmac_context, signal_buffer **output, void *user_data)
{
    int result = 0;
    unsigned char md[EVP_MAX_MD_SIZE];
    unsigned int len = 0;
    HMAC_CTX *ctx = hmac_context;

    if(HMAC_Final(ctx, md, &len) != 1) {
        return SG_ERR_UNKNOWN;
    }

    signal_buffer *output_buffer = signal_buffer_create(md, len);
    if(!output_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    *output = output_buffer;

complete:
    return result;
}

void test_hmac_sha256_cleanup(void *hmac_context, void *user_data)
{
    if(hmac_context) {
        HMAC_CTX *ctx = hmac_context;
#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
        HMAC_CTX_free(ctx);
#else
        HMAC_CTX_cleanup(ctx);
        free(ctx);
#endif
    }
}

const EVP_CIPHER *aes_cipher(int cipher, size_t key_len)
{
    if(cipher == SG_CIPHER_AES_CBC_PKCS5) {
        if(key_len == 16) {
            return EVP_aes_128_cbc();
        }
        else if(key_len == 24) {
            return EVP_aes_192_cbc();
        }
        else if(key_len == 32) {
            return EVP_aes_256_cbc();
        }
    }
    else if(cipher == SG_CIPHER_AES_CTR_NOPADDING) {
        if(key_len == 16) {
            return EVP_aes_128_ctr();
        }
        else if(key_len == 24) {
            return EVP_aes_192_ctr();
        }
        else if(key_len == 32) {
            return EVP_aes_256_ctr();
        }
    }
    return 0;
}

int test_sha512_digest_init(void **digest_context, void *user_data)
{
    int result = 0;
    EVP_MD_CTX *ctx;

    ctx = EVP_MD_CTX_create();
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    result = EVP_DigestInit_ex(ctx, EVP_sha512(), 0);
    if(result == 1) {
        result = SG_SUCCESS;
    }
    else {
        result = SG_ERR_UNKNOWN;
    }

complete:
    if(result < 0) {
        if(ctx) {
            EVP_MD_CTX_destroy(ctx);
        }
    }
    else {
        *digest_context = ctx;
    }
    return result;
}

int test_sha512_digest_update(void *digest_context, const uint8_t *data, size_t data_len, void *user_data)
{
    EVP_MD_CTX *ctx = digest_context;

    int result = EVP_DigestUpdate(ctx, data, data_len);

    return (result == 1) ? SG_SUCCESS : SG_ERR_UNKNOWN;
}

int test_sha512_digest_final(void *digest_context, signal_buffer **output, void *user_data)
{
    int result = 0;
    unsigned char md[EVP_MAX_MD_SIZE];
    unsigned int len = 0;
    EVP_MD_CTX *ctx = digest_context;

    result = EVP_DigestFinal_ex(ctx, md, &len);
    if(result == 1) {
        result = SG_SUCCESS;
    }
    else {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = EVP_DigestInit_ex(ctx, EVP_sha512(), 0);
    if(result == 1) {
        result = SG_SUCCESS;
    }
    else {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    signal_buffer *output_buffer = signal_buffer_create(md, len);
    if(!output_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    *output = output_buffer;

complete:
    return result;
}

void test_sha512_digest_cleanup(void *digest_context, void *user_data)
{
    EVP_MD_CTX *ctx = digest_context;
    EVP_MD_CTX_destroy(ctx);
}

int test_encrypt(signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *plaintext, size_t plaintext_len,
        void *user_data)
{
    int result = 0;
    EVP_CIPHER_CTX *ctx = 0;
    uint8_t *out_buf = 0;

    const EVP_CIPHER *evp_cipher = aes_cipher(cipher, key_len);
    if(!evp_cipher) {
        fprintf(stderr, "invalid AES mode or key size: %zu\n", key_len);
        return SG_ERR_UNKNOWN;
    }

    if(iv_len != 16) {
        fprintf(stderr, "invalid AES IV size: %zu\n", iv_len);
        return SG_ERR_UNKNOWN;
    }

    if(plaintext_len > INT_MAX - EVP_CIPHER_block_size(evp_cipher)) {
        fprintf(stderr, "invalid plaintext length: %zu\n", plaintext_len);
        return SG_ERR_UNKNOWN;
    }

#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
    ctx = EVP_CIPHER_CTX_new();
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
#else
    ctx = malloc(sizeof(EVP_CIPHER_CTX));
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    EVP_CIPHER_CTX_init(ctx);
#endif

    result = EVP_EncryptInit_ex(ctx, evp_cipher, 0, key, iv);
    if(!result) {
        fprintf(stderr, "cannot initialize cipher\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    if(cipher == SG_CIPHER_AES_CTR_NOPADDING) {
        result = EVP_CIPHER_CTX_set_padding(ctx, 0);
        if(!result) {
            fprintf(stderr, "cannot set padding\n");
            result = SG_ERR_UNKNOWN;
            goto complete;
        }
    }

    out_buf = malloc(sizeof(uint8_t) * (plaintext_len + EVP_CIPHER_block_size(evp_cipher)));
    if(!out_buf) {
        fprintf(stderr, "cannot allocate output buffer\n");
        result = SG_ERR_NOMEM;
        goto complete;
    }

    int out_len = 0;
    result = EVP_EncryptUpdate(ctx,
        out_buf, &out_len, plaintext, plaintext_len);
    if(!result) {
        fprintf(stderr, "cannot encrypt plaintext\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    int final_len = 0;
    result = EVP_EncryptFinal_ex(ctx, out_buf + out_len, &final_len);
    if(!result) {
        fprintf(stderr, "cannot finish encrypting plaintext\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    *output = signal_buffer_create(out_buf, out_len + final_len);

complete:
    if(ctx) {
#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
        EVP_CIPHER_CTX_free(ctx);
#else
        EVP_CIPHER_CTX_cleanup(ctx);
        free(ctx);
#endif
    }
    if(out_buf) {
        free(out_buf);
    }
    return result;
}

int test_decrypt(signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *ciphertext, size_t ciphertext_len,
        void *user_data)
{
    int result = 0;
    EVP_CIPHER_CTX *ctx = 0;
    uint8_t *out_buf = 0;

    const EVP_CIPHER *evp_cipher = aes_cipher(cipher, key_len);
    if(!evp_cipher) {
        fprintf(stderr, "invalid AES mode or key size: %zu\n", key_len);
        return SG_ERR_INVAL;
    }

    if(iv_len != 16) {
        fprintf(stderr, "invalid AES IV size: %zu\n", iv_len);
        return SG_ERR_INVAL;
    }

    if(ciphertext_len > INT_MAX - EVP_CIPHER_block_size(evp_cipher)) {
        fprintf(stderr, "invalid ciphertext length: %zu\n", ciphertext_len);
        return SG_ERR_UNKNOWN;
    }

#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
    ctx = EVP_CIPHER_CTX_new();
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
#else
    ctx = malloc(sizeof(EVP_CIPHER_CTX));
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    EVP_CIPHER_CTX_init(ctx);
#endif

    result = EVP_DecryptInit_ex(ctx, evp_cipher, 0, key, iv);
    if(!result) {
        fprintf(stderr, "cannot initialize cipher\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    if(cipher == SG_CIPHER_AES_CTR_NOPADDING) {
        result = EVP_CIPHER_CTX_set_padding(ctx, 0);
        if(!result) {
            fprintf(stderr, "cannot set padding\n");
            result = SG_ERR_UNKNOWN;
            goto complete;
        }
    }

    out_buf = malloc(sizeof(uint8_t) * (ciphertext_len + EVP_CIPHER_block_size(evp_cipher)));
    if(!out_buf) {
        fprintf(stderr, "cannot allocate output buffer\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    int out_len = 0;
    result = EVP_DecryptUpdate(ctx,
        out_buf, &out_len, ciphertext, ciphertext_len);
    if(!result) {
        fprintf(stderr, "cannot decrypt ciphertext\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    int final_len = 0;
    result = EVP_DecryptFinal_ex(ctx, out_buf + out_len, &final_len);
    if(!result) {
        fprintf(stderr, "cannot finish decrypting ciphertext\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    *output = signal_buffer_create(out_buf, out_len + final_len);

complete:
    if(ctx) {
#if OPENSSL_VERSION_NUMBER >= 0x1010000fL
        EVP_CIPHER_CTX_free(ctx);
#else
        EVP_CIPHER_CTX_cleanup(ctx);
        free(ctx);
#endif
    }
    if(out_buf) {
        free(out_buf);
    }
    return result;
}
