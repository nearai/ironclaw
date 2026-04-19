#include "sender_key_state.h"

#include <stdlib.h>
#include <string.h>
#include <assert.h>

#include "sender_key.h"
#include "utlist.h"
#include "LocalStorageProtocol.pb-c.h"
#include "signal_protocol_internal.h"

#define MAX_MESSAGE_KEYS 2000

typedef struct sender_message_key_node {
    sender_message_key *key;
    struct sender_message_key_node *prev, *next;
} sender_message_key_node;

struct sender_key_state
{
    signal_type_base base;

    uint32_t key_id;
    sender_chain_key *chain_key;
    ec_public_key *signature_public_key;
    ec_private_key *signature_private_key;
    sender_message_key_node *message_keys_head;

    signal_context *global_context;
};

int sender_key_state_create(sender_key_state **state,
        uint32_t id, sender_chain_key *chain_key,
        ec_public_key *signature_public_key, ec_private_key *signature_private_key,
        signal_context *global_context)
{
    sender_key_state *result = 0;

    if(!chain_key || !signature_public_key) {
        return SG_ERR_INVAL;
    }

    result = malloc(sizeof(sender_key_state));
    if(!result) {
        return SG_ERR_NOMEM;
    }
    memset(result, 0, sizeof(sender_key_state));
    SIGNAL_INIT(result, sender_key_state_destroy);

    result->key_id = id;

    SIGNAL_REF(chain_key);
    result->chain_key = chain_key;

    SIGNAL_REF(signature_public_key);
    result->signature_public_key = signature_public_key;

    if(signature_private_key) {
        SIGNAL_REF(signature_private_key);
        result->signature_private_key = signature_private_key;
    }

    result->global_context = global_context;

    *state = result;
    return 0;
}

int sender_key_state_serialize(signal_buffer **buffer, sender_key_state *state)
{
    int result = 0;
    size_t result_size = 0;
    uint8_t *data;
    size_t len;
    Textsecure__SenderKeyStateStructure *state_structure = 0;
    signal_buffer *result_buf = 0;

    state_structure = malloc(sizeof(Textsecure__SenderKeyStateStructure));
    if(!state_structure) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    textsecure__sender_key_state_structure__init(state_structure);

    result = sender_key_state_serialize_prepare(state, state_structure);
    if(result < 0) {
        goto complete;
    }

    len = textsecure__sender_key_state_structure__get_packed_size(state_structure);

    result_buf = signal_buffer_alloc(len);
    if(!result_buf) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_buf);
    result_size = textsecure__sender_key_state_structure__pack(state_structure, data);
    if(result_size != len) {
        signal_buffer_free(result_buf);
        result = SG_ERR_INVALID_PROTO_BUF;
        result_buf = 0;
        goto complete;
    }

complete:
    if(state_structure) {
        sender_key_state_serialize_prepare_free(state_structure);
    }
    if(result >= 0) {
        *buffer = result_buf;
    }
    return result;
}

int sender_key_state_deserialize(sender_key_state **state, const uint8_t *data, size_t len, signal_context *global_context)
{
    int result = 0;
    Textsecure__SenderKeyStateStructure *state_structure = 0;
    sender_key_state *result_state = 0;

    state_structure = textsecure__sender_key_state_structure__unpack(0, len, data);
    if(!state_structure) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    result = sender_key_state_deserialize_protobuf(&result_state, state_structure, global_context);
    if(result < 0) {
        goto complete;
    }

complete:
    if(state_structure) {
        textsecure__sender_key_state_structure__free_unpacked(state_structure, 0);
    }
    if(result_state) {
        if(result < 0) {
            SIGNAL_UNREF(result_state);
        }
        else {
            *state = result_state;
        }
    }

    return result;
}

int sender_key_state_serialize_prepare(sender_key_state *state, Textsecure__SenderKeyStateStructure *state_structure)
{
    int result = 0;
    size_t i = 0;
    Textsecure__SenderKeyStateStructure__SenderChainKey *chain_key_structure = 0;
    Textsecure__SenderKeyStateStructure__SenderSigningKey *signing_key_structure = 0;
    sender_message_key_node *cur_node = 0;
    signal_buffer *chain_key_seed = 0;

    assert(state);
    assert(state_structure);

    /* Sender key ID */
    state_structure->has_senderkeyid = 1;
    state_structure->senderkeyid = state->key_id;

    /* Sender chain key */
    chain_key_structure = malloc(sizeof(Textsecure__SenderKeyStateStructure__SenderChainKey));
    if(!chain_key_structure) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    textsecure__sender_key_state_structure__sender_chain_key__init(chain_key_structure);
    state_structure->senderchainkey = chain_key_structure;

    chain_key_structure->iteration = sender_chain_key_get_iteration(state->chain_key);
    chain_key_structure->has_iteration = 1;

    chain_key_seed = sender_chain_key_get_seed(state->chain_key);
    chain_key_structure->seed.data = signal_buffer_data(chain_key_seed);
    chain_key_structure->seed.len = signal_buffer_len(chain_key_seed);
    chain_key_structure->has_seed = 1;

    /* Sender signing key */
    signing_key_structure = malloc(sizeof(Textsecure__SenderKeyStateStructure__SenderSigningKey));
    if(!signing_key_structure) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    textsecure__sender_key_state_structure__sender_signing_key__init(signing_key_structure);
    state_structure->sendersigningkey = signing_key_structure;

    if(state->signature_public_key) {
        result = ec_public_key_serialize_protobuf(&(signing_key_structure->public_), state->signature_public_key);
        if(result < 0) {
            goto complete;
        }
        signing_key_structure->has_public_ = 1;
    }

    if(state->signature_private_key) {
        result = ec_private_key_serialize_protobuf(&(signing_key_structure->private_), state->signature_private_key);
        if(result < 0) {
            goto complete;
        }
        signing_key_structure->has_private_ = 1;
    }

    /* Sender message keys */
    if(state->message_keys_head) {
        size_t count;
        DL_COUNT(state->message_keys_head, cur_node, count);

        if(count > SIZE_MAX / sizeof(Textsecure__SenderKeyStateStructure__SenderMessageKey *)) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        state_structure->sendermessagekeys = malloc(sizeof(Textsecure__SenderKeyStateStructure__SenderMessageKey *) * count);
        if(!state_structure->sendermessagekeys) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        i = 0;
        DL_FOREACH(state->message_keys_head, cur_node) {
            signal_buffer *seed = 0;
            state_structure->sendermessagekeys[i] = malloc(sizeof(Textsecure__SenderKeyStateStructure__SenderMessageKey));
            if(!state_structure->sendermessagekeys[i]) {
                result = SG_ERR_NOMEM;
                break;
            }
            textsecure__sender_key_state_structure__sender_message_key__init(state_structure->sendermessagekeys[i]);

            state_structure->sendermessagekeys[i]->iteration = sender_message_key_get_iteration(cur_node->key);
            state_structure->sendermessagekeys[i]->has_iteration = 1;

            seed = sender_message_key_get_seed(cur_node->key);
            state_structure->sendermessagekeys[i]->seed.data = signal_buffer_data(seed);
            state_structure->sendermessagekeys[i]->seed.len = signal_buffer_len(seed);
            state_structure->sendermessagekeys[i]->has_seed = 1;

            if(result < 0) {
                break;
            }
            i++;
        }
        state_structure->n_sendermessagekeys = i;
        if(result < 0) {
            goto complete;
        }
    }

complete:
    return result;
}

void sender_key_state_serialize_prepare_free(Textsecure__SenderKeyStateStructure *state_structure)
{
    unsigned int i = 0;
    if(state_structure->senderchainkey) {
        free(state_structure->senderchainkey);
    }
    if(state_structure->sendersigningkey) {
        if(state_structure->sendersigningkey->public_.data) {
            free(state_structure->sendersigningkey->public_.data);
        }
        if(state_structure->sendersigningkey->private_.data) {
            free(state_structure->sendersigningkey->private_.data);
        }
        free(state_structure->sendersigningkey);
    }

    if(state_structure->sendermessagekeys) {
        for(i = 0; i < state_structure->n_sendermessagekeys; i++) {
            if(state_structure->sendermessagekeys[i]) {
                free(state_structure->sendermessagekeys[i]);
            }
        }
        free(state_structure->sendermessagekeys);
    }
    free(state_structure);
}

int sender_key_state_deserialize_protobuf(sender_key_state **state, Textsecure__SenderKeyStateStructure *state_structure, signal_context *global_context)
{
    int result = 0;
    sender_key_state *result_state = 0;
    sender_chain_key *chain_key = 0;
    ec_public_key *signature_public_key = 0;
    ec_private_key *signature_private_key = 0;

    if(state_structure->senderchainkey
            && state_structure->senderchainkey->has_iteration
            && state_structure->senderchainkey->has_seed) {
        signal_buffer *seed_buffer = signal_buffer_create(
                state_structure->senderchainkey->seed.data,
                state_structure->senderchainkey->seed.len);
        if(!seed_buffer) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        result = sender_chain_key_create(&chain_key,
                state_structure->senderchainkey->iteration,
                seed_buffer,
                global_context);
        signal_buffer_free(seed_buffer);
        if(result < 0) {
            goto complete;
        }
    }

    if(state_structure->sendersigningkey) {
        if(state_structure->sendersigningkey->has_public_) {
            result = curve_decode_point(&signature_public_key,
                    state_structure->sendersigningkey->public_.data,
                    state_structure->sendersigningkey->public_.len,
                    global_context);
            if(result < 0) {
                goto complete;
            }
        }
        if(state_structure->sendersigningkey->has_private_) {
            result = curve_decode_private_point(&signature_private_key,
                    state_structure->sendersigningkey->private_.data,
                    state_structure->sendersigningkey->private_.len,
                    global_context);
            if(result < 0) {
                goto complete;
            }
        }
    }

    if(state_structure->has_senderkeyid && chain_key && signature_public_key) {
        unsigned int i;
        result = sender_key_state_create(&result_state,
                state_structure->senderkeyid, chain_key,
                signature_public_key, signature_private_key,
                global_context);
        if(result < 0) {
            goto complete;
        }

        if(state_structure->n_sendermessagekeys > 0) {
            for(i = 0; i < state_structure->n_sendermessagekeys; i++) {
                signal_buffer *seed_buffer;
                sender_message_key *message_key;
                Textsecure__SenderKeyStateStructure__SenderMessageKey *message_key_structure =
                        state_structure->sendermessagekeys[i];

                if(!message_key_structure->has_iteration || !message_key_structure->has_seed) {
                    continue;
                }

                seed_buffer = signal_buffer_create(
                        message_key_structure->seed.data,
                        message_key_structure->seed.len);
                if(!seed_buffer) {
                    result = SG_ERR_NOMEM;
                    goto complete;
                }

                result = sender_message_key_create(&message_key,
                        message_key_structure->iteration, seed_buffer,
                        global_context);
                signal_buffer_free(seed_buffer);
                if(result < 0) {
                    goto complete;
                }

                result = sender_key_state_add_sender_message_key(result_state, message_key);
                if(result < 0) {
                    goto complete;
                }
                SIGNAL_UNREF(message_key);
            }
        }
    }
    else {
        result = SG_ERR_INVALID_PROTO_BUF;
    }

complete:
    if(chain_key) {
        SIGNAL_UNREF(chain_key);
    }
    if(signature_public_key) {
        SIGNAL_UNREF(signature_public_key);
    }
    if(signature_private_key) {
        SIGNAL_UNREF(signature_private_key);
    }
    if(result >= 0) {
        *state = result_state;
    }
    else {
        if(result_state) {
            SIGNAL_UNREF(result_state);
        }
    }
    return result;
}

int sender_key_state_copy(sender_key_state **state, sender_key_state *other_state, signal_context *global_context)
{
    int result = 0;
    signal_buffer *buffer = 0;
    uint8_t *data;
    size_t len;

    assert(other_state);
    assert(global_context);

    result = sender_key_state_serialize(&buffer, other_state);
    if(result < 0) {
        goto complete;
    }

    data = signal_buffer_data(buffer);
    len = signal_buffer_len(buffer);

    result = sender_key_state_deserialize(state, data, len, global_context);
    if(result < 0) {
        goto complete;
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    return result;
}

uint32_t sender_key_state_get_key_id(sender_key_state *state)
{
    assert(state);
    return state->key_id;
}

sender_chain_key *sender_key_state_get_chain_key(sender_key_state *state)
{
    assert(state);
    return state->chain_key;
}

void sender_key_state_set_chain_key(sender_key_state *state, sender_chain_key *chain_key)
{
    assert(state);
    assert(chain_key);

    if(state->chain_key) {
        SIGNAL_UNREF(state->chain_key);
    }
    SIGNAL_REF(chain_key);
    state->chain_key = chain_key;
}

ec_public_key *sender_key_state_get_signing_key_public(sender_key_state *state)
{
    assert(state);
    return state->signature_public_key;
}

ec_private_key *sender_key_state_get_signing_key_private(sender_key_state *state)
{
    assert(state);
    return state->signature_private_key;
}

int sender_key_state_has_sender_message_key(sender_key_state *state, uint32_t iteration)
{
    sender_message_key_node *cur_node = 0;
    assert(state);

    DL_FOREACH(state->message_keys_head, cur_node) {
        if(sender_message_key_get_iteration(cur_node->key) == iteration) {
            return 1;
        }
    }

    return 0;
}

int sender_key_state_add_sender_message_key(sender_key_state *state, sender_message_key *message_key)
{
    int result = 0;
    sender_message_key_node *node = 0;
    int count;
    assert(state);
    assert(message_key);

    node = malloc(sizeof(sender_message_key_node));
    if(!node) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    SIGNAL_REF(message_key);
    node->key = message_key;
    DL_APPEND(state->message_keys_head, node);

    DL_COUNT(state->message_keys_head, node, count);
    while(count > MAX_MESSAGE_KEYS) {
        node = state->message_keys_head;
        DL_DELETE(state->message_keys_head, node);
        if(node->key) {
            SIGNAL_UNREF(node->key);
        }
        free(node);
        --count;
    }

complete:
    return result;
}

sender_message_key *sender_key_state_remove_sender_message_key(sender_key_state *state, uint32_t iteration)
{
    sender_message_key *result = 0;
    sender_message_key_node *cur_node = 0;
    sender_message_key_node *tmp_node = 0;
    assert(state);

    DL_FOREACH_SAFE(state->message_keys_head, cur_node, tmp_node) {
        if(sender_message_key_get_iteration(cur_node->key) == iteration) {
            DL_DELETE(state->message_keys_head, cur_node);
            result = cur_node->key;
            free(cur_node);
            break;
        }
    }

    return result;
}

void sender_key_state_destroy(signal_type_base *type)
{
    sender_key_state *state = (sender_key_state *)type;
    sender_message_key_node *cur_node;
    sender_message_key_node *tmp_node;

    SIGNAL_UNREF(state->chain_key);
    SIGNAL_UNREF(state->signature_public_key);
    SIGNAL_UNREF(state->signature_private_key);

    DL_FOREACH_SAFE(state->message_keys_head, cur_node, tmp_node) {
        DL_DELETE(state->message_keys_head, cur_node);
        if(cur_node->key) {
            SIGNAL_UNREF(cur_node->key);
        }
        free(cur_node);
    }
    state->message_keys_head = 0;

    free(state);
}
