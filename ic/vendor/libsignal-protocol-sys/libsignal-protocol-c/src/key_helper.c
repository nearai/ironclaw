#include "key_helper.h"

#include <assert.h>

#include "session_pre_key.h"
#include "ratchet.h"
#include "curve.h"
#include "signal_protocol_internal.h"
#include "utlist.h"

struct signal_protocol_key_helper_pre_key_list_node
{
    session_pre_key *element;
    struct signal_protocol_key_helper_pre_key_list_node *next;
};

int signal_protocol_key_helper_generate_identity_key_pair(ratchet_identity_key_pair **key_pair, signal_context *global_context)
{
    int result = 0;
    ratchet_identity_key_pair *result_pair = 0;
    ec_key_pair *ec_pair = 0;
    ec_public_key *public_key = 0;
    ec_private_key *private_key = 0;

    assert(global_context);

    result = curve_generate_key_pair(global_context, &ec_pair);
    if(result < 0) {
        goto complete;
    }

    public_key = ec_key_pair_get_public(ec_pair);
    private_key = ec_key_pair_get_private(ec_pair);

    result = ratchet_identity_key_pair_create(
            &result_pair, public_key, private_key);

complete:
    if(result >= 0) {
        *key_pair = result_pair;
    }
    SIGNAL_UNREF(ec_pair);
    return result;
}

int signal_protocol_key_helper_generate_registration_id(uint32_t *registration_id, int extended_range, signal_context *global_context)
{
    uint32_t range;
    uint32_t id_value;
    int result = 0;

    assert(global_context);
    assert(global_context->crypto_provider.random_func);

    if(extended_range == 0) {
        range = 16380;
    }
    else if(extended_range == 1) {
        range = INT32_MAX - 1;
    }
    else {
        return SG_ERR_INVAL;
    }

    result = global_context->crypto_provider.random_func(
            (uint8_t *)(&id_value), sizeof(id_value),
            global_context->crypto_provider.user_data);
    if(result < 0) {
        return result;
    }

    id_value = (id_value % range) + 1;

    *registration_id = id_value;

    return 0;
}

int signal_protocol_key_helper_get_random_sequence(int *value, int max, signal_context *global_context)
{
    int result = 0;
    int32_t result_value;

    assert(global_context);
    assert(global_context->crypto_provider.random_func);

    result = global_context->crypto_provider.random_func(
            (uint8_t *)(&result_value), sizeof(result_value),
            global_context->crypto_provider.user_data);
    if(result < 0) {
        return result;
    }

    result_value = ((result_value & 0x7FFFFFFF) % max);

    *value = result_value;

    return 0;
}

int signal_protocol_key_helper_generate_pre_keys(signal_protocol_key_helper_pre_key_list_node **head,
        unsigned int start, unsigned int count,
        signal_context *global_context)
{
    int result = 0;
    ec_key_pair *ec_pair = 0;
    session_pre_key *pre_key = 0;
    signal_protocol_key_helper_pre_key_list_node *result_head = 0;
    signal_protocol_key_helper_pre_key_list_node *cur_node = 0;
    signal_protocol_key_helper_pre_key_list_node *node = 0;
    unsigned int start_index = start - 1;
    unsigned int i;

    assert(global_context);

    for(i = 0; i < count; i++) {
        uint32_t id = 0;
        result = curve_generate_key_pair(global_context, &ec_pair);
        if(result < 0) {
            goto complete;
        }

        id = ((start_index + i) % (PRE_KEY_MEDIUM_MAX_VALUE - 1)) + 1;

        result = session_pre_key_create(&pre_key, id, ec_pair);
        if(result < 0) {
            goto complete;
        }

        SIGNAL_UNREF(ec_pair);
        ec_pair = 0;

        node = malloc(sizeof(signal_protocol_key_helper_pre_key_list_node));
        if(!node) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
        node->element = pre_key;
        node->next = 0;
        if(!result_head) {
            result_head = node;
            cur_node = node;
        }
        else {
            cur_node->next = node;
            cur_node = node;
        }
        pre_key = 0;
        node = 0;
    }

complete:
    if(ec_pair) {
        SIGNAL_UNREF(ec_pair);
    }
    if(pre_key) {
        SIGNAL_UNREF(pre_key);
    }
    if(node) {
        free(node);
    }
    if(result < 0) {
        if(result_head) {
            signal_protocol_key_helper_pre_key_list_node *tmp_node;
            LL_FOREACH_SAFE(result_head, cur_node, tmp_node) {
                LL_DELETE(result_head, cur_node);
                SIGNAL_UNREF(cur_node->element);
                free(cur_node);
            }
        }
    }
    else {
        *head = result_head;
    }
    return result;
}

session_pre_key *signal_protocol_key_helper_key_list_element(const signal_protocol_key_helper_pre_key_list_node *node)
{
    assert(node);
    assert(node->element);
    return node->element;
}

signal_protocol_key_helper_pre_key_list_node *signal_protocol_key_helper_key_list_next(const signal_protocol_key_helper_pre_key_list_node *node)
{
    assert(node);
    return node->next;
}

void signal_protocol_key_helper_key_list_free(signal_protocol_key_helper_pre_key_list_node *head)
{
    if(head) {
        signal_protocol_key_helper_pre_key_list_node *cur_node;
        signal_protocol_key_helper_pre_key_list_node *tmp_node;
        LL_FOREACH_SAFE(head, cur_node, tmp_node) {
            LL_DELETE(head, cur_node);
            SIGNAL_UNREF(cur_node->element);
            free(cur_node);
        }
    }
}

int signal_protocol_key_helper_generate_signed_pre_key(session_signed_pre_key **signed_pre_key,
        const ratchet_identity_key_pair *identity_key_pair,
        uint32_t signed_pre_key_id,
        uint64_t timestamp,
        signal_context *global_context)
{
    int result = 0;
    session_signed_pre_key *result_signed_pre_key = 0;
    ec_key_pair *ec_pair = 0;
    signal_buffer *public_buf = 0;
    signal_buffer *signature_buf = 0;
    ec_public_key *public_key = 0;
    ec_private_key *private_key = 0;

    assert(global_context);

    result = curve_generate_key_pair(global_context, &ec_pair);
    if(result < 0) {
        goto complete;
    }

    public_key = ec_key_pair_get_public(ec_pair);
    result = ec_public_key_serialize(&public_buf, public_key);
    if(result < 0) {
        goto complete;
    }

    private_key = ratchet_identity_key_pair_get_private(identity_key_pair);

    result = curve_calculate_signature(global_context,
            &signature_buf,
            private_key,
            signal_buffer_data(public_buf),
            signal_buffer_len(public_buf));
    if(result < 0) {
        goto complete;
    }

    result = session_signed_pre_key_create(&result_signed_pre_key,
            signed_pre_key_id, timestamp, ec_pair,
            signal_buffer_data(signature_buf),
            signal_buffer_len(signature_buf));

complete:
    SIGNAL_UNREF(ec_pair);
    signal_buffer_free(public_buf);
    signal_buffer_free(signature_buf);
    if(result >= 0) {
        *signed_pre_key = result_signed_pre_key;
    }
    return result;
}

int signal_protocol_key_helper_generate_sender_signing_key(ec_key_pair **key_pair, signal_context *global_context)
{
    int result;

    assert(global_context);

    result = curve_generate_key_pair(global_context, key_pair);

    return result;
}

int signal_protocol_key_helper_generate_sender_key(signal_buffer **key_buffer, signal_context *global_context)
{
    int result = 0;
    signal_buffer *result_buffer = 0;

    assert(global_context);

    result_buffer = signal_buffer_alloc(32);
    if(!result_buffer) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    result = signal_crypto_random(global_context,
            signal_buffer_data(result_buffer),
            signal_buffer_len(result_buffer));

complete:
    if(result < 0) {
        signal_buffer_free(result_buffer);
    }
    else {
        *key_buffer = result_buffer;
        result = 0;
    }
    return result;
}

int signal_protocol_key_helper_generate_sender_key_id(uint32_t *key_id, signal_context *global_context)
{
    int result;
    int value;

    result = signal_protocol_key_helper_get_random_sequence(&value, INT32_MAX, global_context);

    if(result >= 0) {
        *key_id = (uint32_t)value;
    }
    return result;
}
