#include "fingerprint.h"

#include <assert.h>
#include <string.h>

#include "FingerprintProtocol.pb-c.h"
#include "signal_protocol_internal.h"
#include "vpool.h"

#define FINGERPRINT_VERSION 0
#define FINGERPRINT_LENGTH 30

#define MAX(a,b) (((a)>(b))?(a):(b))

struct fingerprint
{
    signal_type_base base;
    displayable_fingerprint *displayable;
    scannable_fingerprint *scannable;
};

struct displayable_fingerprint
{
    signal_type_base base;
    char *local_fingerprint;
    char *remote_fingerprint;
    char *display_text;
};

struct scannable_fingerprint
{
    signal_type_base base;
    uint32_t version;
    char *local_stable_identifier;
    signal_buffer *local_fingerprint;
    char *remote_stable_identifier;
    signal_buffer *remote_fingerprint;
};

struct fingerprint_generator
{
    int iterations;
    int scannable_version;
    signal_context *global_context;
};

static int fingerprint_generator_get_logical_key_bytes(signal_buffer **key_bytes,
        const ec_public_key_list *unsorted_key_list);

static int fingerprint_generator_create_for_impl(fingerprint_generator *generator,
        const char *local_stable_identifier, const signal_buffer *local_identity_buffer,
        const char *remote_stable_identifier, const signal_buffer *remote_identity_buffer,
        fingerprint **fingerprint_val);

static int fingerprint_generator_get_fingerprint(fingerprint_generator *generator, signal_buffer **fingerprint_buffer,
        const char *stable_identifier, const signal_buffer *identity_buffer);

static int fingerprint_generator_create_display_string(fingerprint_generator *generator,
        char **display_string, signal_buffer *fingerprint_buffer);

int fingerprint_generator_create(fingerprint_generator **generator,
        int iterations, int scannable_version,
        signal_context *global_context)
{
    fingerprint_generator *result_generator;

    assert(global_context);

    if(scannable_version < 0 || scannable_version > 1) {
        return SG_ERR_INVAL;
    }

    result_generator = malloc(sizeof(fingerprint_generator));
    if(!result_generator) {
        return SG_ERR_NOMEM;
    }
    memset(result_generator, 0, sizeof(fingerprint_generator));

    result_generator->iterations = iterations;
    result_generator->scannable_version = scannable_version;
    result_generator->global_context = global_context;

    *generator = result_generator;
    return 0;
}

int fingerprint_generator_create_for(fingerprint_generator *generator,
        const char *local_stable_identifier, const ec_public_key *local_identity_key,
        const char *remote_stable_identifier, const ec_public_key *remote_identity_key,
        fingerprint **fingerprint_val)
{
    int result = 0;
    signal_buffer *local_key_buffer = 0;
    signal_buffer *remote_key_buffer = 0;

    result = ec_public_key_serialize(&local_key_buffer, local_identity_key);
    if(result < 0) {
        goto complete;
    }

    result = ec_public_key_serialize(&remote_key_buffer, remote_identity_key);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_create_for_impl(generator,
            local_stable_identifier, local_key_buffer,
            remote_stable_identifier, remote_key_buffer,
            fingerprint_val);

complete:
    signal_buffer_free(local_key_buffer);
    signal_buffer_free(remote_key_buffer);
    return result;
}

int fingerprint_generator_create_for_list(fingerprint_generator *generator,
        const char *local_stable_identifier, const ec_public_key_list *local_identity_key_list,
        const char *remote_stable_identifier, const ec_public_key_list *remote_identity_key_list,
        fingerprint **fingerprint_val)
{
    int result = 0;
    signal_buffer *local_key_buffer = 0;
    signal_buffer *remote_key_buffer = 0;

    result = fingerprint_generator_get_logical_key_bytes(&local_key_buffer, local_identity_key_list);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_get_logical_key_bytes(&remote_key_buffer, remote_identity_key_list);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_create_for_impl(generator,
            local_stable_identifier, local_key_buffer,
            remote_stable_identifier, remote_key_buffer,
            fingerprint_val);

complete:
    signal_buffer_free(local_key_buffer);
    signal_buffer_free(remote_key_buffer);
    return result;
}

int fingerprint_generator_create_for_impl(fingerprint_generator *generator,
        const char *local_stable_identifier, const signal_buffer *local_identity_buffer,
        const char *remote_stable_identifier, const signal_buffer *remote_identity_buffer,
        fingerprint **fingerprint_val)
{
    int result = 0;
    fingerprint *result_fingerprint = 0;
    signal_buffer *local_fingerprint_buffer = 0;
    signal_buffer *remote_fingerprint_buffer = 0;
    displayable_fingerprint *displayable = 0;
    char *displayable_local = 0;
    char *displayable_remote = 0;
    scannable_fingerprint *scannable = 0;

    result = fingerprint_generator_get_fingerprint(generator,
            &local_fingerprint_buffer, local_stable_identifier, local_identity_buffer);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_get_fingerprint(generator,
            &remote_fingerprint_buffer, remote_stable_identifier, remote_identity_buffer);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_create_display_string(generator, &displayable_local,
            local_fingerprint_buffer);
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_generator_create_display_string(generator, &displayable_remote,
            remote_fingerprint_buffer);
    if(result < 0) {
        goto complete;
    }

    result = displayable_fingerprint_create(&displayable, displayable_local, displayable_remote);
    if(result < 0) {
        goto complete;
    }

    if(generator->scannable_version == 0) {
        result = scannable_fingerprint_create(&scannable, 0,
                local_stable_identifier, local_identity_buffer,
                remote_stable_identifier, remote_identity_buffer);
    }
    else if(generator->scannable_version == 1) {
        result = scannable_fingerprint_create(&scannable, 1,
                0, local_fingerprint_buffer,
                0, remote_fingerprint_buffer);
    }
    else {
        result = SG_ERR_INVAL;
    }
    if(result < 0) {
        goto complete;
    }

    result = fingerprint_create(&result_fingerprint, displayable, scannable);

complete:
    signal_buffer_free(local_fingerprint_buffer);
    signal_buffer_free(remote_fingerprint_buffer);
    if(displayable_local) {
        free(displayable_local);
    }
    if(displayable_remote) {
        free(displayable_remote);
    }
    SIGNAL_UNREF(displayable);
    SIGNAL_UNREF(scannable);
    if(result >= 0) {
        *fingerprint_val = result_fingerprint;
    }
    return result;
}

int fingerprint_generator_get_logical_key_bytes(signal_buffer **key_bytes,
        const ec_public_key_list *unsorted_key_list)
{
    int result = 0;
    ec_public_key_list *sorted_key_list = 0;
    ec_public_key *key_element = 0;
    unsigned int list_size = 0;
    unsigned int i = 0;
    struct vpool vp;
    signal_buffer *buffer = 0;

    vpool_init(&vp, 1024, 0);

    sorted_key_list = ec_public_key_list_copy(unsorted_key_list);
    if(!sorted_key_list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    ec_public_key_list_sort(sorted_key_list);
    list_size = ec_public_key_list_size(sorted_key_list);

    for(i = 0; i < list_size; i++) {
        key_element = ec_public_key_list_at(sorted_key_list, i);

        result = ec_public_key_serialize(&buffer, key_element);
        if (result < 0) {
            goto complete;
        }

        if(!vpool_insert(&vp, vpool_get_length(&vp),
                signal_buffer_data(buffer), signal_buffer_len(buffer))) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        signal_buffer_free(buffer);
        buffer = 0;
    }

    buffer = signal_buffer_create(vpool_get_buf(&vp), vpool_get_length(&vp));
    if(!buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

complete:
    ec_public_key_list_free(sorted_key_list);
    vpool_final(&vp);
    if(result >= 0) {
        *key_bytes = buffer;
    }
    else {
        signal_buffer_free(buffer);
    }
    return result;
}

int fingerprint_generator_get_fingerprint(fingerprint_generator *generator, signal_buffer **fingerprint_buffer,
        const char *stable_identifier, const signal_buffer *identity_buffer)
{
    int result = 0;
    void *digest_context = 0;
    signal_buffer *hash_buffer = 0;
    signal_buffer *hash_out_buffer = 0;
    uint8_t *data = 0;
    size_t len = 0;
    int i = 0;

    assert(generator);
    assert(stable_identifier);
    assert(identity_buffer);

    result = signal_sha512_digest_init(generator->global_context, &digest_context);
    if(result < 0) {
        goto complete;
    }

    len = 2 + signal_buffer_len(identity_buffer) + strlen(stable_identifier);

    hash_buffer = signal_buffer_alloc(len);
    if(!hash_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(hash_buffer);

    memset(data, 0, len);

    data[0] = 0;
    data[1] = (uint8_t)FINGERPRINT_VERSION;
    memcpy(data + 2, signal_buffer_const_data(identity_buffer), signal_buffer_len(identity_buffer));
    memcpy(data + 2 + signal_buffer_len(identity_buffer), stable_identifier, strlen(stable_identifier));

    for(i = 0; i < generator->iterations; i++) {
        data = signal_buffer_data(hash_buffer);
        len = signal_buffer_len(hash_buffer);

        result = signal_sha512_digest_update(generator->global_context,
                digest_context, data, len);
        if(result < 0) {
            goto complete;
        }

        result = signal_sha512_digest_update(generator->global_context,
                digest_context,
                signal_buffer_const_data(identity_buffer),
                signal_buffer_len(identity_buffer));
        if(result < 0) {
            goto complete;
        }

        result = signal_sha512_digest_final(generator->global_context,
                digest_context, &hash_out_buffer);
        if(result < 0) {
            goto complete;
        }

        signal_buffer_free(hash_buffer);
        hash_buffer = hash_out_buffer;
        hash_out_buffer = 0;
    }

    len = signal_buffer_len(hash_buffer);

    if(len < FINGERPRINT_LENGTH) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

complete:
    if(digest_context) {
        signal_sha512_digest_cleanup(generator->global_context, digest_context);
    }
    if(result >= 0) {
        *fingerprint_buffer = hash_buffer;
    }
    else {
        signal_buffer_free(hash_buffer);
    }
    return result;
}

int fingerprint_generator_create_display_string(fingerprint_generator *generator,
        char **display_string, signal_buffer *fingerprint_buffer)
{
    int result = 0;
    char *result_string = 0;
    uint8_t *data = 0;
    size_t len = 0;
    int i = 0;

    assert(generator);
    assert(fingerprint_buffer);

    data = signal_buffer_data(fingerprint_buffer);
    len = signal_buffer_len(fingerprint_buffer);

    if(len < FINGERPRINT_LENGTH) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    result_string = malloc(FINGERPRINT_LENGTH+1);
    if(!result_string) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    for(i = 0; i < FINGERPRINT_LENGTH; i += 5) {
        uint64_t chunk = ((uint64_t)data[i] & 0xFFL) << 32 |
                ((uint64_t)data[i + 1] & 0xFFL) << 24 |
                ((uint64_t)data[i + 2] & 0xFFL) << 16 |
                ((uint64_t)data[i + 3] & 0xFFL) << 8 |
                ((uint64_t)data[i + 4] & 0xFFL);
#if _WINDOWS
        sprintf_s(result_string + i, 6, "%05d", (int)(chunk % 100000));
#else
        snprintf(result_string + i, 6, "%05d", (int)(chunk % 100000));
#endif
    }

complete:
    if(result >= 0) {
        *display_string = result_string;
    }
    return result;
}

void fingerprint_generator_free(fingerprint_generator *generator)
{
    if(generator) {
        free(generator);
    }
}

int fingerprint_create(fingerprint **fingerprint_val, displayable_fingerprint *displayable, scannable_fingerprint *scannable)
{
    fingerprint *result = malloc(sizeof(fingerprint));
    if(!result) {
        return SG_ERR_NOMEM;
    }

    memset(result, 0, sizeof(fingerprint));
    SIGNAL_INIT(result, fingerprint_destroy);
    if(displayable) {
        result->displayable = displayable;
        SIGNAL_REF(displayable);
    }
    if(scannable) {
        result->scannable = scannable;
        SIGNAL_REF(scannable);
    }

    *fingerprint_val = result;

    return 0;
}

displayable_fingerprint *fingerprint_get_displayable(const fingerprint *fingerprint_val)
{
    assert(fingerprint_val);
    return fingerprint_val->displayable;
}

scannable_fingerprint *fingerprint_get_scannable(const fingerprint *fingerprint_val)
{
    assert(fingerprint_val);
    return fingerprint_val->scannable;
}

void fingerprint_destroy(signal_type_base *type)
{
    fingerprint *fingerprint_val = (fingerprint *)type;
    SIGNAL_UNREF(fingerprint_val->displayable);
    SIGNAL_UNREF(fingerprint_val->scannable);
    free(fingerprint_val);
}

int displayable_fingerprint_create(displayable_fingerprint **displayable, const char *local_fingerprint, const char *remote_fingerprint)
{
    int result = 0;
    size_t local_len = 0;
    size_t remote_len = 0;
    displayable_fingerprint *result_displayable = 0;
    char *display_text = 0;

    if(!local_fingerprint || !remote_fingerprint) {
        return SG_ERR_INVAL;
    }

    result_displayable = malloc(sizeof(displayable_fingerprint));
    if(!result_displayable) {
        return SG_ERR_NOMEM;
    }

    memset(result_displayable, 0, sizeof(displayable_fingerprint));
    SIGNAL_INIT(result_displayable, displayable_fingerprint_destroy);

    result_displayable->local_fingerprint = strdup(local_fingerprint);
    if(!result_displayable->local_fingerprint) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    result_displayable->remote_fingerprint = strdup(remote_fingerprint);
    if(!result_displayable->remote_fingerprint) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    local_len = strlen(local_fingerprint);
    remote_len = strlen(remote_fingerprint);

    display_text = malloc(local_len + remote_len + 1);
    if(!display_text) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    if(strcmp(local_fingerprint, remote_fingerprint) <= 0) {
        memcpy(display_text, local_fingerprint, local_len);
        memcpy(display_text + local_len, remote_fingerprint, remote_len + 1);
    }
    else {
        memcpy(display_text, remote_fingerprint, remote_len);
        memcpy(display_text + remote_len, local_fingerprint, local_len + 1);
    }

    result_displayable->display_text = display_text;

complete:
    if(result < 0) {
        SIGNAL_UNREF(result_displayable);
    }
    else {
        *displayable = result_displayable;
    }

    return result;
}

const char *displayable_fingerprint_local(const displayable_fingerprint *displayable)
{
    assert(displayable);
    return displayable->local_fingerprint;
}

const char *displayable_fingerprint_remote(const displayable_fingerprint *displayable)
{
    assert(displayable);
    return displayable->remote_fingerprint;
}

const char *displayable_fingerprint_text(const displayable_fingerprint *displayable)
{
    assert(displayable);
    return displayable->display_text;
}

void displayable_fingerprint_destroy(signal_type_base *type)
{
    displayable_fingerprint *displayable = (displayable_fingerprint *)type;
    if(displayable->local_fingerprint) {
        free(displayable->local_fingerprint);
    }
    if(displayable->remote_fingerprint) {
        free(displayable->remote_fingerprint);
    }
    if(displayable->display_text) {
        free(displayable->display_text);
    }
    free(displayable);
}

int scannable_fingerprint_create(scannable_fingerprint **scannable,
        uint32_t version,
        const char *local_stable_identifier, const signal_buffer *local_fingerprint,
        const char *remote_stable_identifier, const signal_buffer *remote_fingerprint)
{
    int result = 0;
    scannable_fingerprint *result_scannable = 0;

    if(version == 0 && (!local_stable_identifier || !remote_stable_identifier)) {
        return SG_ERR_INVAL;
    }

    if(!local_fingerprint || !remote_fingerprint) {
        return SG_ERR_INVAL;
    }

    result_scannable = malloc(sizeof(scannable_fingerprint));
    if(!result_scannable) {
        return SG_ERR_NOMEM;
    }

    memset(result_scannable, 0, sizeof(scannable_fingerprint));
    SIGNAL_INIT(result_scannable, scannable_fingerprint_destroy);

    result_scannable->version = version;

    if(version == 0 && local_stable_identifier) {
        result_scannable->local_stable_identifier = strdup(local_stable_identifier);
        if(!result_scannable->local_stable_identifier) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
    }

    if(version == 0) {
        result_scannable->local_fingerprint = signal_buffer_copy(local_fingerprint);
    }
    else {
        result_scannable->local_fingerprint = signal_buffer_n_copy(local_fingerprint, 32);
    }
    if(!result_scannable->local_fingerprint) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    if(version == 0 && remote_stable_identifier) {
        result_scannable->remote_stable_identifier = strdup(remote_stable_identifier);
        if(!result_scannable->remote_stable_identifier) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
    }

    if(version == 0) {
        result_scannable->remote_fingerprint = signal_buffer_copy(remote_fingerprint);
    }
    else {
        result_scannable->remote_fingerprint = signal_buffer_n_copy(remote_fingerprint, 32);
    }
    if(!result_scannable->remote_fingerprint) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

complete:
    if(result < 0) {
        SIGNAL_UNREF(result_scannable);
    }
    else {
        *scannable = result_scannable;
    }

    return result;
}

int scannable_fingerprint_serialize(signal_buffer **buffer, const scannable_fingerprint *scannable)
{
    int result = 0;
    size_t result_size = 0;
    signal_buffer *result_buf = 0;
    Textsecure__CombinedFingerprints combined_fingerprint = TEXTSECURE__COMBINED_FINGERPRINTS__INIT;
    Textsecure__LogicalFingerprint local_fingerprint = TEXTSECURE__LOGICAL_FINGERPRINT__INIT;
    Textsecure__LogicalFingerprint remote_fingerprint = TEXTSECURE__LOGICAL_FINGERPRINT__INIT;
    size_t len = 0;
    uint8_t *data = 0;

    combined_fingerprint.version = scannable->version;
    combined_fingerprint.has_version = 1;

    if(scannable->local_fingerprint) {
        if(scannable->version == 0 && scannable->local_stable_identifier) {
            signal_protocol_str_serialize_protobuf(&local_fingerprint.identifier, scannable->local_stable_identifier);
            local_fingerprint.has_identifier = 1;
        }

        local_fingerprint.content.data = signal_buffer_data(scannable->local_fingerprint);
        local_fingerprint.content.len = signal_buffer_len(scannable->local_fingerprint);
        local_fingerprint.has_content = 1;

        combined_fingerprint.localfingerprint = &local_fingerprint;
    }

    if(scannable->remote_fingerprint) {
        if(scannable->version == 0 && scannable->remote_stable_identifier) {
            signal_protocol_str_serialize_protobuf(&remote_fingerprint.identifier, scannable->remote_stable_identifier);
            remote_fingerprint.has_identifier = 1;
        }

        remote_fingerprint.content.data = signal_buffer_data(scannable->remote_fingerprint);
        remote_fingerprint.content.len = signal_buffer_len(scannable->remote_fingerprint);
        remote_fingerprint.has_content = 1;

        combined_fingerprint.remotefingerprint = &remote_fingerprint;
    }

    len = textsecure__combined_fingerprints__get_packed_size(&combined_fingerprint);

    result_buf = signal_buffer_alloc(len);
    if(!result_buf) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_buf);
    result_size = textsecure__combined_fingerprints__pack(&combined_fingerprint, data);
    if(result_size != len) {
        signal_buffer_free(result_buf);
        result = SG_ERR_INVALID_PROTO_BUF;
        result_buf = 0;
        goto complete;
    }

complete:
    if(result >= 0) {
        *buffer = result_buf;
    }
    return result;
}

int scannable_fingerprint_deserialize(scannable_fingerprint **scannable, const uint8_t *data, size_t len, signal_context *global_context)
{
    int result = 0;
    Textsecure__CombinedFingerprints *combined_fingerprint = 0;
    uint32_t version = 0;
    char *local_stable_identifier = 0;
    signal_buffer *local_fingerprint = 0;
    char *remote_stable_identifier = 0;
    signal_buffer *remote_fingerprint = 0;

    combined_fingerprint = textsecure__combined_fingerprints__unpack(0, len, data);
    if(!combined_fingerprint) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    if(combined_fingerprint->has_version) {
        version = combined_fingerprint->version;
    }

    if(combined_fingerprint->localfingerprint) {
        if(combined_fingerprint->localfingerprint->has_identifier) {
            local_stable_identifier = signal_protocol_str_deserialize_protobuf(&combined_fingerprint->localfingerprint->identifier);
            if(!local_stable_identifier) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
        }
        if(combined_fingerprint->localfingerprint->has_content) {
            local_fingerprint = signal_buffer_create(
                    combined_fingerprint->localfingerprint->content.data,
                    combined_fingerprint->localfingerprint->content.len);
            if(!local_fingerprint) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
        }
    }

    if(combined_fingerprint->remotefingerprint) {
        if(combined_fingerprint->remotefingerprint->has_identifier) {
            remote_stable_identifier = signal_protocol_str_deserialize_protobuf(&combined_fingerprint->remotefingerprint->identifier);
            if(!remote_stable_identifier) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
        }
        if(combined_fingerprint->remotefingerprint->has_content) {
            remote_fingerprint = signal_buffer_create(
                    combined_fingerprint->remotefingerprint->content.data,
                    combined_fingerprint->remotefingerprint->content.len);
            if(!remote_fingerprint) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
        }
    }

    result = scannable_fingerprint_create(scannable, version,
            local_stable_identifier, local_fingerprint,
            remote_stable_identifier, remote_fingerprint);

complete:
    if(combined_fingerprint) {
        textsecure__combined_fingerprints__free_unpacked(combined_fingerprint, 0);
    }
    if(local_stable_identifier) {
        free(local_stable_identifier);
    }
    if(remote_stable_identifier) {
        free(remote_stable_identifier);
    }
    signal_buffer_free(local_fingerprint);
    signal_buffer_free(remote_fingerprint);
    return result;
}

uint32_t scannable_fingerprint_get_version(const scannable_fingerprint *scannable)
{
    assert(scannable);
    return scannable->version;
}

const char *scannable_fingerprint_get_local_stable_identifier(const scannable_fingerprint *scannable)
{
    assert(scannable);
    return scannable->local_stable_identifier;
}

signal_buffer *scannable_fingerprint_get_local_fingerprint(const scannable_fingerprint *scannable)
{
    assert(scannable);
    return scannable->local_fingerprint;
}

const char *scannable_fingerprint_get_remote_stable_identifier(const scannable_fingerprint *scannable)
{
    assert(scannable);
    return scannable->remote_stable_identifier;
}

signal_buffer *scannable_fingerprint_get_remote_fingerprint(const scannable_fingerprint *scannable)
{
    assert(scannable);
    return scannable->remote_fingerprint;
}

int scannable_fingerprint_compare(const scannable_fingerprint *scannable, const scannable_fingerprint *other_scannable)
{
    if(!other_scannable->remote_fingerprint || !other_scannable->local_fingerprint ||
            other_scannable->version != scannable->version) {
        return SG_ERR_FP_VERSION_MISMATCH;
    }

    if(scannable->version == 0) {
        if(strcmp(scannable->local_stable_identifier, other_scannable->remote_stable_identifier) != 0) {
            return SG_ERR_FP_IDENT_MISMATCH;
        }

        if(strcmp(scannable->remote_stable_identifier, other_scannable->local_stable_identifier) != 0) {
            return SG_ERR_FP_IDENT_MISMATCH;
        }
    }

    if(signal_buffer_compare(scannable->local_fingerprint, other_scannable->remote_fingerprint) != 0) {
        return 0;
    }

    if(signal_buffer_compare(scannable->remote_fingerprint, other_scannable->local_fingerprint) != 0) {
        return 0;
    }

    return 1;
}

void scannable_fingerprint_destroy(signal_type_base *type)
{
    scannable_fingerprint *scannable = (scannable_fingerprint *)type;

    if(scannable->local_stable_identifier) {
        free(scannable->local_stable_identifier);
    }
    if(scannable->remote_stable_identifier) {
        free(scannable->remote_stable_identifier);
    }

    signal_buffer_free(scannable->local_fingerprint);
    signal_buffer_free(scannable->remote_fingerprint);

    free(scannable);
}
