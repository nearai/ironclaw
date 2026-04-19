#include "test_common.h"

#include <CommonCrypto/CommonDigest.h>
#include <CommonCrypto/CommonHMAC.h>
#include <CommonCrypto/CommonCryptor.h>

#include <stdio.h>

int test_random_generator(uint8_t *data, size_t len, void *user_data)
{
    arc4random_buf(data, len);
    return 0;

#if 0
    /*
     * Apple's documentation recommends this method for generating secure
     * random numbers. However, it is too slow for the purpose of unit tests.
     */
    int result = 0;

    FILE *fp = fopen("/dev/random", "r");
    if(!fp) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    size_t n = fread(data, 1, len, fp);
    if(n != len) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

complete:
    if(fp) {
        fclose(fp);
    }
    return result;
#endif
}

int test_hmac_sha256_init(void **hmac_context, const uint8_t *key, size_t key_len, void *user_data)
{
    CCHmacContext *ctx = malloc(sizeof(CCHmacContext));
    if(!ctx) {
        return SG_ERR_NOMEM;
    }

    CCHmacInit(ctx, kCCHmacAlgSHA256, key, key_len);
    *hmac_context = ctx;

    return 0;
}

int test_hmac_sha256_update(void *hmac_context, const uint8_t *data, size_t data_len, void *user_data)
{
    CCHmacContext *ctx = hmac_context;
    CCHmacUpdate(ctx, data, data_len);
    return 0;
}

int test_hmac_sha256_final(void *hmac_context, signal_buffer **output, void *user_data)
{
    CCHmacContext *ctx = hmac_context;

    signal_buffer *output_buffer = signal_buffer_alloc(CC_SHA256_DIGEST_LENGTH);
    if(!output_buffer) {
        return SG_ERR_NOMEM;
    }

    CCHmacFinal(ctx, signal_buffer_data(output_buffer));

    *output = output_buffer;

    return 0;
}

void test_hmac_sha256_cleanup(void *hmac_context, void *user_data)
{
    if(hmac_context) {
        CCHmacContext *ctx = hmac_context;
        free(ctx);
    }
}

int test_sha512_digest_init(void **digest_context, void *user_data)
{
    int result = 0;

    CC_SHA512_CTX *ctx = malloc(sizeof(CC_SHA512_CTX));
    if(!ctx) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    result = CC_SHA512_Init(ctx);
    if(result != 1) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

complete:
    if(result < 0) {
        if(ctx) {
            free(ctx);
        }
    }
    else {
        *digest_context = ctx;
    }
    return result;
}

int test_sha512_digest_update(void *digest_context, const uint8_t *data, size_t data_len, void *user_data)
{
    CC_SHA512_CTX *ctx = digest_context;

    int result = CC_SHA512_Update(ctx, data, data_len);

    return (result == 1) ? SG_SUCCESS : SG_ERR_UNKNOWN;
}

int test_sha512_digest_final(void *digest_context, signal_buffer **output, void *user_data)
{
    int result = 0;
    unsigned char md[CC_SHA512_DIGEST_LENGTH];
    CC_SHA512_CTX *ctx = digest_context;

    result = CC_SHA512_Final(md, ctx);
    if(result == 1) {
        result = SG_SUCCESS;
    }
    else {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result = CC_SHA512_Init(ctx);
    if(result == 1) {
        result = SG_SUCCESS;
    }
    else {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    signal_buffer *output_buffer = signal_buffer_create(md, CC_SHA512_DIGEST_LENGTH);
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
    if(digest_context) {
        CC_SHA512_CTX *ctx = digest_context;
        free(ctx);
    }
}

int cc_status_to_result(CCCryptorStatus status)
{
    switch(status) {
    case kCCSuccess:
        return SG_SUCCESS;
    case kCCParamError:
    case kCCBufferTooSmall:
        return SG_ERR_INVAL;
    case kCCMemoryFailure:
        return SG_ERR_NOMEM;
    case kCCAlignmentError:
    case kCCDecodeError:
    case kCCUnimplemented:
    case kCCOverflow:
    case kCCRNGFailure:
    case kCCUnspecifiedError:
    case kCCCallSequenceError:
    default:
        return SG_ERR_UNKNOWN;
    }
}

int test_encrypt(signal_buffer **output,
        int cipher,
        const uint8_t *key, size_t key_len,
        const uint8_t *iv, size_t iv_len,
        const uint8_t *plaintext, size_t plaintext_len,
        void *user_data)
{
    int result = 0;
    uint8_t *out_buf = 0;
    CCCryptorStatus status = kCCSuccess;
    CCCryptorRef ref = 0;

    if(cipher == SG_CIPHER_AES_CBC_PKCS5) {
        status = CCCryptorCreate(kCCEncrypt, kCCAlgorithmAES, kCCOptionPKCS7Padding, key, key_len, iv, &ref);
    }
    else if(cipher == SG_CIPHER_AES_CTR_NOPADDING) {
        status = CCCryptorCreateWithMode(kCCEncrypt, kCCModeCTR, kCCAlgorithmAES, ccNoPadding,
                iv, key, key_len, 0, 0, 0, kCCModeOptionCTR_BE, &ref);
    }
    else {
        status = kCCParamError;
    }
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    size_t available_len = CCCryptorGetOutputLength(ref, plaintext_len, 1);
    out_buf = malloc(available_len);
    if(!out_buf) {
        fprintf(stderr, "cannot allocate output buffer\n");
        result = SG_ERR_NOMEM;
        goto complete;
    }

    size_t update_moved_len = 0;
    status = CCCryptorUpdate(ref, plaintext, plaintext_len, out_buf, available_len, &update_moved_len);
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    size_t final_moved_len = 0;
    status = CCCryptorFinal(ref, out_buf + update_moved_len, available_len - update_moved_len, &final_moved_len);
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    signal_buffer *output_buffer = signal_buffer_create(out_buf, update_moved_len + final_moved_len);
    if(!output_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    *output = output_buffer;

complete:
    if(ref) {
        CCCryptorRelease(ref);
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
    uint8_t *out_buf = 0;
    CCCryptorStatus status = kCCSuccess;
    CCCryptorRef ref = 0;

    if(cipher == SG_CIPHER_AES_CBC_PKCS5) {
        status = CCCryptorCreate(kCCDecrypt, kCCAlgorithmAES, kCCOptionPKCS7Padding, key, key_len, iv, &ref);
    }
    else if(cipher == SG_CIPHER_AES_CTR_NOPADDING) {
        status = CCCryptorCreateWithMode(kCCDecrypt, kCCModeCTR, kCCAlgorithmAES, ccNoPadding,
                iv, key, key_len, 0, 0, 0, kCCModeOptionCTR_BE, &ref);
    }
    else {
        status = kCCParamError;
    }
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    out_buf = malloc(sizeof(uint8_t) * ciphertext_len);
    if(!out_buf) {
        fprintf(stderr, "cannot allocate output buffer\n");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    size_t update_moved_len = 0;
    status = CCCryptorUpdate(ref, ciphertext, ciphertext_len, out_buf, ciphertext_len, &update_moved_len);
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    size_t final_moved_len = 0;
    status = CCCryptorFinal(ref, out_buf + update_moved_len, ciphertext_len - update_moved_len, &final_moved_len);
    if(status != kCCSuccess) {
        result = cc_status_to_result(status);
        goto complete;
    }

    signal_buffer *output_buffer = signal_buffer_create(out_buf, update_moved_len + final_moved_len);
    if(!output_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    *output = output_buffer;

complete:
    if(ref) {
        CCCryptorRelease(ref);
    }
    if(out_buf) {
        free(out_buf);
    }
    return result;
}
