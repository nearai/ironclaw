#include "device_consistency.h"

#include <assert.h>
#include <string.h>

#include "signal_protocol_internal.h"
#include "curve.h"
#include "WhisperTextProtocol.pb-c.h"
#include "signal_utarray.h"

#define CODE_VERSION 0

struct device_consistency_signature
{
    signal_type_base base;
    signal_buffer *signature;
    signal_buffer *vrf_output;
};

struct device_consistency_commitment
{
    signal_type_base base;
    uint32_t generation;
    signal_buffer *serialized;
};

struct device_consistency_message
{
    signal_type_base base;
    device_consistency_signature *signature;
    uint32_t generation;
    signal_buffer *serialized;
};

struct device_consistency_signature_list
{
    UT_array *values;
};

static int device_consistency_message_create(device_consistency_message **message);
static void device_consistency_signature_list_sort(device_consistency_signature_list *list);

/*------------------------------------------------------------------------*/

int device_consistency_signature_create(device_consistency_signature **signature,
        const uint8_t *signature_data, size_t signature_len,
        const uint8_t *vrf_output_data, size_t vrf_output_len)
{
    int result = 0;
    device_consistency_signature *result_signature = 0;

    result_signature = malloc(sizeof(device_consistency_signature));
    if(!result_signature) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memset(result_signature, 0, sizeof(device_consistency_signature));
    SIGNAL_INIT(result_signature, device_consistency_signature_destroy);

    result_signature->signature = signal_buffer_create(signature_data, signature_len);
    if(!result_signature->signature) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    result_signature->vrf_output = signal_buffer_create(vrf_output_data, vrf_output_len);
    if(!result_signature->vrf_output) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

complete:
    if(result >= 0) {
        *signature = result_signature;
    }
    else {
        SIGNAL_UNREF(result_signature);
    }
    return result;
}

signal_buffer *device_consistency_signature_get_signature(const device_consistency_signature *signature)
{
    assert(signature);
    return signature->signature;
}

signal_buffer *device_consistency_signature_get_vrf_output(const device_consistency_signature *signature)
{
    assert(signature);
    return signature->vrf_output;
}

void device_consistency_signature_destroy(signal_type_base *type)
{
    device_consistency_signature *signature = (device_consistency_signature *)type;
    signal_buffer_free(signature->signature);
    signal_buffer_free(signature->vrf_output);
    free(signature);
}

/*------------------------------------------------------------------------*/

int device_consistency_commitment_create(device_consistency_commitment **commitment,
        uint32_t generation, ec_public_key_list *identity_key_list,
        signal_context *global_context)
{
    static const char version[] = "DeviceConsistencyCommitment_V0";
    int result = 0;
    void *digest_context = 0;
    device_consistency_commitment *result_commitment = 0;
    ec_public_key_list *sorted_list = 0;
    uint8_t gen_data[4];
    unsigned int list_size;
    unsigned int i;

    result_commitment = malloc(sizeof(device_consistency_commitment));
    if(!result_commitment) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memset(result_commitment, 0, sizeof(device_consistency_commitment));
    SIGNAL_INIT(result_commitment, device_consistency_commitment_destroy);

    sorted_list = ec_public_key_list_copy(identity_key_list);
    if(!sorted_list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    ec_public_key_list_sort(sorted_list);

    result = signal_sha512_digest_init(global_context, &digest_context);
    if(result < 0) {
        goto complete;
    }

    result = signal_sha512_digest_update(global_context, digest_context,
            (uint8_t *)version, sizeof(version) - 1);
    if(result < 0) {
        goto complete;
    }

    gen_data[3] = (uint8_t)(generation);
    gen_data[2] = (uint8_t)(generation >> 8);
    gen_data[1] = (uint8_t)(generation >> 16);
    gen_data[0] = (uint8_t)(generation >> 24);

    result = signal_sha512_digest_update(global_context, digest_context,
            gen_data, sizeof(gen_data));
    if(result < 0) {
        goto complete;
    }

    list_size = ec_public_key_list_size(sorted_list);
    for(i = 0; i < list_size; i++) {
        signal_buffer *key_buffer = 0;
        ec_public_key *key = ec_public_key_list_at(sorted_list, i);

        result = ec_public_key_serialize(&key_buffer, key);
        if(result < 0) {
            goto complete;
        }

        result = signal_sha512_digest_update(global_context, digest_context,
                signal_buffer_data(key_buffer), signal_buffer_len(key_buffer));
        signal_buffer_free(key_buffer);
        if(result < 0) {
            goto complete;
        }
    }

    result_commitment->generation = generation;
    result = signal_sha512_digest_final(global_context, digest_context, &result_commitment->serialized);

complete:
    if(sorted_list) {
        ec_public_key_list_free(sorted_list);
    }
    if(digest_context) {
        signal_sha512_digest_cleanup(global_context, digest_context);
    }
    if(result >= 0) {
        *commitment = result_commitment;
    }
    else {
        SIGNAL_UNREF(result_commitment);
    }
    return result;
}

uint32_t device_consistency_commitment_get_generation(const device_consistency_commitment *commitment)
{
    assert(commitment);
    return commitment->generation;
}

signal_buffer *device_consistency_commitment_get_serialized(const device_consistency_commitment *commitment)
{
    assert(commitment);
    return commitment->serialized;
}

void device_consistency_commitment_destroy(signal_type_base *type)
{
    device_consistency_commitment *commitment = (device_consistency_commitment *)type;
    signal_buffer_free(commitment->serialized);
    free(commitment);
}

/*------------------------------------------------------------------------*/

int device_consistency_message_create(device_consistency_message **message)
{
    int result = 0;
    device_consistency_message *result_message = 0;

    result_message = malloc(sizeof(device_consistency_message));
    if(!result_message) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memset(result_message, 0, sizeof(device_consistency_message));
    SIGNAL_INIT(result_message, device_consistency_message_destroy);

complete:
    if(result >= 0) {
        *message = result_message;
    }
    return result;
}

int device_consistency_message_create_from_pair(device_consistency_message **message,
        device_consistency_commitment *commitment,
        ec_key_pair *identity_key_pair,
        signal_context *global_context)
{
    int result = 0;
    device_consistency_message *result_message = 0;
    signal_buffer *commitment_buffer = 0;
    signal_buffer *signature_buffer = 0;
    signal_buffer *vrf_output_buffer = 0;
    signal_buffer *serialized_signature_buffer = 0;
    Textsecure__DeviceConsistencyCodeMessage message_structure = TEXTSECURE__DEVICE_CONSISTENCY_CODE_MESSAGE__INIT;
    size_t len = 0;
    uint8_t *data = 0;
    size_t result_size = 0;

    /* Create message instance */
    result = device_consistency_message_create(&result_message);
    if(result < 0) {
        goto complete;
    }

    /* Calculate VRF signature */
    commitment_buffer = device_consistency_commitment_get_serialized(commitment);
    result = curve_calculate_vrf_signature(global_context, &signature_buffer,
            ec_key_pair_get_private(identity_key_pair),
            signal_buffer_data(commitment_buffer), signal_buffer_len(commitment_buffer));
    if(result < 0) {
        goto complete;
    }

    /* Verify VRF signature */
    result = curve_verify_vrf_signature(global_context, &vrf_output_buffer,
            ec_key_pair_get_public(identity_key_pair),
            signal_buffer_data(commitment_buffer), signal_buffer_len(commitment_buffer),
            signal_buffer_data(signature_buffer), signal_buffer_len(signature_buffer));
    if(result < 0) {
        goto complete;
    }

    result_message->generation = device_consistency_commitment_get_generation(commitment);

    /* Create and assign the signature */
    result = device_consistency_signature_create(&result_message->signature,
            signal_buffer_data(signature_buffer), signal_buffer_len(signature_buffer),
            signal_buffer_data(vrf_output_buffer), signal_buffer_len(vrf_output_buffer));
    if(result < 0) {
        goto complete;
    }

    serialized_signature_buffer = device_consistency_signature_get_signature(result_message->signature);

    /* Serialize the message */
    message_structure.generation = device_consistency_commitment_get_generation(commitment);
    message_structure.has_generation = 1;
    message_structure.signature.data = signal_buffer_data(serialized_signature_buffer);
    message_structure.signature.len = signal_buffer_len(serialized_signature_buffer);
    message_structure.has_signature = 1;

    len = textsecure__device_consistency_code_message__get_packed_size(&message_structure);
    result_message->serialized = signal_buffer_alloc(len);
    if(!result_message->serialized) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_message->serialized);

    result_size = textsecure__device_consistency_code_message__pack(&message_structure, data);
    if(result_size != len) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

complete:
    signal_buffer_free(signature_buffer);
    signal_buffer_free(vrf_output_buffer);
    if(result >= 0) {
        *message = result_message;
    }
    else {
        SIGNAL_UNREF(result_message);
    }
    if(result == SG_ERR_INVALID_KEY || result == SG_ERR_VRF_SIG_VERIF_FAILED) {
        result = SG_ERR_UNKNOWN;
    }
    return result;
}

int device_consistency_message_create_from_serialized(device_consistency_message **message,
        device_consistency_commitment *commitment,
        const uint8_t *serialized_data, size_t serialized_len,
        ec_public_key *identity_key,
        signal_context *global_context)
{
    int result = 0;
    device_consistency_message *result_message = 0;
    Textsecure__DeviceConsistencyCodeMessage *message_structure = 0;
    signal_buffer *commitment_buffer = 0;
    signal_buffer *vrf_output_buffer = 0;

    /* Create message instance */
    result = device_consistency_message_create(&result_message);
    if(result < 0) {
        goto complete;
    }

    /* Deserialize the message */
    message_structure = textsecure__device_consistency_code_message__unpack(0, serialized_len, serialized_data);
    if(!message_structure) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    if(!message_structure->has_generation || !message_structure->has_signature) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    /* Verify VRF signature */
    commitment_buffer = device_consistency_commitment_get_serialized(commitment);
    result = curve_verify_vrf_signature(global_context, &vrf_output_buffer,
            identity_key,
            signal_buffer_data(commitment_buffer), signal_buffer_len(commitment_buffer),
            message_structure->signature.data, message_structure->signature.len);
    if(result < 0) {
        goto complete;
    }

    /* Assign the message fields */
    result_message->generation = message_structure->generation;

    result = device_consistency_signature_create(&result_message->signature,
            message_structure->signature.data, message_structure->signature.len,
            signal_buffer_data(vrf_output_buffer), signal_buffer_len(vrf_output_buffer));
    if(result < 0) {
        goto complete;
    }

    result_message->serialized = signal_buffer_create(serialized_data, serialized_len);
    if(!result_message->serialized) {
        result = SG_ERR_NOMEM;
    }

complete:
    if(message_structure) {
        textsecure__device_consistency_code_message__free_unpacked(message_structure, 0);
    }
    signal_buffer_free(vrf_output_buffer);
    if(result >= 0) {
        *message = result_message;
    }
    else {
        SIGNAL_UNREF(result_message);
    }
    if(result == SG_ERR_INVALID_PROTO_BUF
            || result == SG_ERR_INVALID_KEY
            || result == SG_ERR_VRF_SIG_VERIF_FAILED) {
        result = SG_ERR_INVALID_MESSAGE;
    }
    return result;
}

signal_buffer *device_consistency_message_get_serialized(const device_consistency_message *message)
{
    assert(message);
    return message->serialized;
}

device_consistency_signature *device_consistency_message_get_signature(const device_consistency_message *message)
{
    assert(message);
    return message->signature;
}

uint32_t device_consistency_signature_get_generation(const device_consistency_message *message)
{
    assert(message);
    return message->generation;
}

void device_consistency_message_destroy(signal_type_base *type)
{
    device_consistency_message *message = (device_consistency_message *)type;
    SIGNAL_UNREF(message->signature);
    signal_buffer_free(message->serialized);
    free(message);
}

/*------------------------------------------------------------------------*/

int device_consistency_code_generate_for(device_consistency_commitment *commitment,
        device_consistency_signature_list *signatures,
        char **code_string,
        signal_context *global_context)
{
    int result = 0;
    char *result_string = 0;
    void *digest_context = 0;
    device_consistency_signature_list *sorted_list = 0;
    uint8_t version_data[2];
    signal_buffer *commitment_buffer;
    unsigned int list_size;
    unsigned int i;
    signal_buffer *hash_buffer = 0;
    uint8_t *data = 0;
    size_t len = 0;
    char *encoded_string = 0;

    sorted_list = device_consistency_signature_list_copy(signatures);
    if(!sorted_list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    device_consistency_signature_list_sort(sorted_list);

    result = signal_sha512_digest_init(global_context, &digest_context);
    if(result < 0) {
        goto complete;
    }

    version_data[1] = (uint8_t)(CODE_VERSION);
    version_data[0] = (uint8_t)(CODE_VERSION >> 8);

    result = signal_sha512_digest_update(global_context, digest_context,
            version_data, sizeof(version_data));
    if(result < 0) {
        goto complete;
    }

    commitment_buffer = device_consistency_commitment_get_serialized(commitment);
    result = signal_sha512_digest_update(global_context, digest_context,
            signal_buffer_data(commitment_buffer),
            signal_buffer_len(commitment_buffer));
    if(result < 0) {
        goto complete;
    }

    list_size = device_consistency_signature_list_size(sorted_list);
    for(i = 0; i < list_size; i++) {
        device_consistency_signature *signature = device_consistency_signature_list_at(sorted_list, i);
        signal_buffer *vrf_output = device_consistency_signature_get_vrf_output(signature);

        result = signal_sha512_digest_update(global_context, digest_context,
                signal_buffer_data(vrf_output),
                signal_buffer_len(vrf_output));
        if(result < 0) {
            goto complete;
        }
    }

    result = signal_sha512_digest_final(global_context, digest_context, &hash_buffer);
    if(result < 0) {
        goto complete;
    }

    data = signal_buffer_data(hash_buffer);
    len = signal_buffer_len(hash_buffer);

    if(len < 10) {
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    encoded_string = malloc(11);
    if(!encoded_string) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    for(i = 0; i < 10; i += 5) {
        uint64_t chunk = ((uint64_t)data[i] & 0xFFL) << 32 |
                ((uint64_t)data[i + 1] & 0xFFL) << 24 |
                ((uint64_t)data[i + 2] & 0xFFL) << 16 |
                ((uint64_t)data[i + 3] & 0xFFL) << 8 |
                ((uint64_t)data[i + 4] & 0xFFL);
#if _WINDOWS
        sprintf_s(encoded_string + i, 6, "%05d", (int)(chunk % 100000));
#else
        snprintf(encoded_string + i, 6, "%05d", (int)(chunk % 100000));
#endif
    }

    result_string = malloc(7);
    if(!result_string) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    memcpy(result_string, encoded_string, 6);
    result_string[6] = '\0';

complete:
    if(sorted_list) {
        device_consistency_signature_list_free(sorted_list);
    }
    if(digest_context) {
        signal_sha512_digest_cleanup(global_context, digest_context);
    }
    signal_buffer_free(hash_buffer);
    free(encoded_string);
    if(result >= 0) {
        *code_string = result_string;
    }
    return result;
}

/*------------------------------------------------------------------------*/

device_consistency_signature_list *device_consistency_signature_list_alloc()
{
    int result = 0;
    device_consistency_signature_list *list = malloc(sizeof(device_consistency_signature_list));
    if(!list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    memset(list, 0, sizeof(device_consistency_signature_list));

    utarray_new(list->values, &ut_ptr_icd);

complete:
    if(result < 0) {
        if(list) {
            free(list);
        }
        return 0;
    }
    else {
        return list;
    }
}

device_consistency_signature_list *device_consistency_signature_list_copy(const device_consistency_signature_list *list)
{
    int result = 0;
    device_consistency_signature_list *result_list = 0;
    unsigned int size;
    unsigned int i;
    device_consistency_signature **p;

    result_list = device_consistency_signature_list_alloc();
    if(!result_list) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    size = utarray_len(list->values);

    utarray_reserve(result_list->values, size);

    for (i = 0; i < size; i++) {
        p = (device_consistency_signature **)utarray_eltptr(list->values, i);
        result = device_consistency_signature_list_push_back(result_list, *p);
        if(result < 0) {
            goto complete;
        }
    }

complete:
    if(result < 0) {
        if(result_list) {
            device_consistency_signature_list_free(result_list);
        }
        return 0;
    }
    else {
        return result_list;
    }
}

int device_consistency_signature_list_push_back(device_consistency_signature_list *list, device_consistency_signature *value)
{
    int result = 0;
    assert(list);
    assert(value);

    utarray_push_back(list->values, &value);
    SIGNAL_REF(value);

complete:
    return result;
}

unsigned int device_consistency_signature_list_size(const device_consistency_signature_list *list)
{
    assert(list);
    return utarray_len(list->values);
}

device_consistency_signature *device_consistency_signature_list_at(const device_consistency_signature_list *list, unsigned int index)
{
    device_consistency_signature **value = 0;

    assert(list);
    assert(index < utarray_len(list->values));

    value = (device_consistency_signature **)utarray_eltptr(list->values, index);

    assert(*value);

    return *value;
}

int device_consistency_signature_list_sort_comparator(const void *a, const void *b)
{
    int result;
    const device_consistency_signature *sig1 = *((const device_consistency_signature **)a);
    const device_consistency_signature *sig2 = *((const device_consistency_signature **)b);
    signal_buffer *buf1 = device_consistency_signature_get_vrf_output(sig1);
    signal_buffer *buf2 = device_consistency_signature_get_vrf_output(sig2);
    size_t len1 = signal_buffer_len(buf1);
    size_t len2 = signal_buffer_len(buf2);

    if(len1 == len2) {
        result = memcmp(signal_buffer_data(buf1), signal_buffer_data(buf2), len1);
    }
    else if (len1 < len2) {
        result = -1;
    } else {
        result = 1;
    }

    return result;
}

void device_consistency_signature_list_sort(device_consistency_signature_list *list)
{
    assert(list);
    utarray_sort(list->values, device_consistency_signature_list_sort_comparator);
}

void device_consistency_signature_list_free(device_consistency_signature_list *list)
{
    unsigned int size;
    unsigned int i;
    device_consistency_signature **p;
    if(list) {
        size = utarray_len(list->values);
        for (i = 0; i < size; i++) {
            p = (device_consistency_signature **)utarray_eltptr(list->values, i);
            SIGNAL_UNREF(*p);
        }
        utarray_free(list->values);
        free(list);
    }
}
