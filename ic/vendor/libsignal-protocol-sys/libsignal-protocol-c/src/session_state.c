#include "session_state.h"

#include <stdlib.h>
#include <string.h>
#include <assert.h>

#include "hkdf.h"
#include "curve.h"
#include "ratchet.h"
#include "LocalStorageProtocol.pb-c.h"
#include "signal_protocol_internal.h"

#include "utlist.h"

#define MAX_MESSAGE_KEYS 2000

typedef struct message_keys_node
{
    ratchet_message_keys message_key;
    struct message_keys_node *prev, *next;
} message_keys_node;

typedef struct session_state_sender_chain
{
    ec_key_pair *sender_ratchet_key_pair;
    ratchet_chain_key *chain_key;
} session_state_sender_chain;

typedef struct session_state_receiver_chain
{
    ec_public_key *sender_ratchet_key;
    ratchet_chain_key *chain_key;
    message_keys_node *message_keys_head;
    struct session_state_receiver_chain *prev, *next;
} session_state_receiver_chain;

typedef struct session_pending_key_exchange
{
    uint32_t sequence;
    ec_key_pair *local_base_key;
    ec_key_pair *local_ratchet_key;
    ratchet_identity_key_pair *local_identity_key;
} session_pending_key_exchange;

typedef struct session_pending_pre_key
{
    int has_pre_key_id;
    uint32_t pre_key_id;
    uint32_t signed_pre_key_id;
    ec_public_key *base_key;
} session_pending_pre_key;

struct session_state
{
    signal_type_base base;

    uint32_t session_version;
    ec_public_key *local_identity_public;
    ec_public_key *remote_identity_public;

    ratchet_root_key *root_key;
    uint32_t previous_counter;

    int has_sender_chain;
    session_state_sender_chain sender_chain;

    session_state_receiver_chain *receiver_chain_head;

    int has_pending_key_exchange;
    session_pending_key_exchange pending_key_exchange;

    int has_pending_pre_key;
    session_pending_pre_key pending_pre_key;

    uint32_t remote_registration_id;
    uint32_t local_registration_id;

    int needs_refresh;
    ec_public_key *alice_base_key;

    signal_context *global_context;
};

static int session_state_serialize_prepare_sender_chain(
        session_state_sender_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure);
static int session_state_serialize_prepare_receiver_chain(
        session_state_receiver_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure);
static void session_state_serialize_prepare_chain_free(
        Textsecure__SessionStructure__Chain *chain_structure);
static int session_state_serialize_prepare_chain_chain_key(
        ratchet_chain_key *chain_key,
        Textsecure__SessionStructure__Chain *chain_structure);
static int session_state_serialize_prepare_chain_message_keys_list(
        message_keys_node *message_keys_head,
        Textsecure__SessionStructure__Chain *chain_structure);
static int session_state_serialize_prepare_message_keys(
        ratchet_message_keys *message_key,
        Textsecure__SessionStructure__Chain__MessageKey *message_key_structure);
static void session_state_serialize_prepare_message_keys_free(
        Textsecure__SessionStructure__Chain__MessageKey *message_key_structure);
static int session_state_serialize_prepare_pending_key_exchange(
        session_pending_key_exchange *exchange,
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure);
static void session_state_serialize_prepare_pending_key_exchange_free(
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure);
static int session_state_serialize_prepare_pending_pre_key(
        session_pending_pre_key *pre_key,
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure);
static void session_state_serialize_prepare_pending_pre_key_free(
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure);

static int session_state_deserialize_protobuf_pending_key_exchange(
        session_pending_key_exchange *result_exchange,
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure,
        signal_context *global_context);
static int session_state_deserialize_protobuf_pending_pre_key(
        session_pending_pre_key *result_pre_key,
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure,
        signal_context *global_context);
static int session_state_deserialize_protobuf_sender_chain(
        uint32_t session_version,
        session_state_sender_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure,
        signal_context *global_context);
static int session_state_deserialize_protobuf_receiver_chain(
        uint32_t session_version,
        session_state_receiver_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure,
        signal_context *global_context);

static void session_state_free_sender_chain(session_state *state);
static void session_state_free_receiver_chain_node(session_state_receiver_chain *node);
static void session_state_free_receiver_chain(session_state *state);
static session_state_receiver_chain *session_state_find_receiver_chain(const session_state *state, const ec_public_key *sender_ephemeral);

int session_state_create(session_state **state, signal_context *global_context)
{
    session_state *result = malloc(sizeof(session_state));
    if(!result) {
        return SG_ERR_NOMEM;
    }
    memset(result, 0, sizeof(session_state));
    SIGNAL_INIT(result, session_state_destroy);
    result->session_version = 2;
    result->global_context = global_context;

    *state = result;
    return 0;
}

int session_state_serialize(signal_buffer **buffer, session_state *state)
{
    int result = 0;
    size_t result_size = 0;
    Textsecure__SessionStructure *state_structure = 0;
    signal_buffer *result_buf = 0;
    size_t len = 0;
    uint8_t *data = 0;

    state_structure = malloc(sizeof(Textsecure__SessionStructure));
    if(!state_structure) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    textsecure__session_structure__init(state_structure);

    result = session_state_serialize_prepare(state, state_structure);
    if(result < 0) {
        goto complete;
    }

    len = textsecure__session_structure__get_packed_size(state_structure);

    result_buf = signal_buffer_alloc(len);
    if(!result_buf) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_buf);
    result_size = textsecure__session_structure__pack(state_structure, data);
    if(result_size != len) {
        signal_buffer_free(result_buf);
        result = SG_ERR_INVALID_PROTO_BUF;
        result_buf = 0;
        goto complete;
    }

complete:
    if(state_structure) {
        session_state_serialize_prepare_free(state_structure);
    }
    if(result >= 0) {
        *buffer = result_buf;
    }
    return result;
}

int session_state_deserialize(session_state **state, const uint8_t *data, size_t len, signal_context *global_context)
{
    int result = 0;
    session_state *result_state = 0;
    Textsecure__SessionStructure *session_structure = 0;

    session_structure = textsecure__session_structure__unpack(0, len, data);
    if(!session_structure) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    result = session_state_deserialize_protobuf(&result_state, session_structure, global_context);
    if(result < 0) {
        goto complete;
    }

complete:
    if(session_structure) {
        textsecure__session_structure__free_unpacked(session_structure, 0);
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

int session_state_serialize_prepare(session_state *state, Textsecure__SessionStructure *session_structure)
{
    int result = 0;

    assert(state);
    assert(session_structure);

    session_structure->has_sessionversion = 1;
    session_structure->sessionversion = state->session_version;

    if(state->local_identity_public) {
        result = ec_public_key_serialize_protobuf(
                &session_structure->localidentitypublic, state->local_identity_public);
        if(result < 0) {
            goto complete;
        }
        session_structure->has_localidentitypublic = 1;
    }

    if(state->remote_identity_public) {
        result = ec_public_key_serialize_protobuf(
                &session_structure->remoteidentitypublic, state->remote_identity_public);
        if(result < 0) {
            goto complete;
        }
        session_structure->has_remoteidentitypublic = 1;
    }

    if(state->root_key) {
        result = ratchet_root_key_get_key_protobuf(
                state->root_key, &session_structure->rootkey);
        if(result < 0) {
            goto complete;
        }
        session_structure->has_rootkey = 1;
    }

    session_structure->has_previouscounter = 1;
    session_structure->previouscounter = state->previous_counter;


    if(state->has_sender_chain) {
        session_structure->senderchain = malloc(sizeof(Textsecure__SessionStructure__Chain));
        if(!session_structure->senderchain) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
        textsecure__session_structure__chain__init(session_structure->senderchain);
        result = session_state_serialize_prepare_sender_chain(
                &state->sender_chain, session_structure->senderchain);
        if(result < 0) {
            goto complete;
        }
    }

    if(state->receiver_chain_head) {
        size_t count, i = 0;
        session_state_receiver_chain *cur_node;
        DL_COUNT(state->receiver_chain_head, cur_node, count);

        if(count > SIZE_MAX / sizeof(Textsecure__SessionStructure__Chain *)) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        session_structure->receiverchains = malloc(sizeof(Textsecure__SessionStructure__Chain *) * count);
        if(!session_structure->receiverchains) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        DL_FOREACH(state->receiver_chain_head, cur_node) {
            session_structure->receiverchains[i] = malloc(sizeof(Textsecure__SessionStructure__Chain));
            if(!session_structure->receiverchains[i]) {
                result = SG_ERR_NOMEM;
                break;
            }
            textsecure__session_structure__chain__init(session_structure->receiverchains[i]);
            result = session_state_serialize_prepare_receiver_chain(cur_node, session_structure->receiverchains[i]);
            if(result < 0) {
                break;
            }
            i++;
        }
        session_structure->n_receiverchains = i;
        if(result < 0) {
            goto complete;
        }
    }

    if(state->has_pending_key_exchange) {
        session_structure->pendingkeyexchange = malloc(sizeof(Textsecure__SessionStructure__PendingKeyExchange));
        if(!session_structure->pendingkeyexchange) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
        textsecure__session_structure__pending_key_exchange__init(session_structure->pendingkeyexchange);
        result = session_state_serialize_prepare_pending_key_exchange(
                &state->pending_key_exchange,
                session_structure->pendingkeyexchange);
        if(result < 0) {
            goto complete;
        }
    }

    if(state->has_pending_pre_key) {
        session_structure->pendingprekey = malloc(sizeof(Textsecure__SessionStructure__PendingPreKey));
        if(!session_structure->pendingprekey) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
        textsecure__session_structure__pending_pre_key__init(session_structure->pendingprekey);
        result = session_state_serialize_prepare_pending_pre_key(
                &state->pending_pre_key,
                session_structure->pendingprekey);
        if(result < 0) {
            goto complete;
        }
    }

    session_structure->has_remoteregistrationid = 1;
    session_structure->remoteregistrationid = state->remote_registration_id;

    session_structure->has_localregistrationid = 1;
    session_structure->localregistrationid = state->local_registration_id;

    session_structure->has_needsrefresh = 1;
    session_structure->needsrefresh = state->needs_refresh;

    if(state->alice_base_key) {
        result = ec_public_key_serialize_protobuf(
                &session_structure->alicebasekey, state->alice_base_key);
        if(result < 0) {
            goto complete;
        }
        session_structure->has_alicebasekey = 1;
    }

complete:
    return result;
}

static int session_state_serialize_prepare_sender_chain(
        session_state_sender_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure)
{
    int result = 0;

    if(chain->sender_ratchet_key_pair) {
        ec_public_key *public_key = 0;
        ec_private_key *private_key = 0;

        public_key = ec_key_pair_get_public(chain->sender_ratchet_key_pair);
        result = ec_public_key_serialize_protobuf(&chain_structure->senderratchetkey, public_key);
        if(result < 0) {
            goto complete;
        }
        chain_structure->has_senderratchetkey = 1;

        private_key = ec_key_pair_get_private(chain->sender_ratchet_key_pair);
        result = ec_private_key_serialize_protobuf(&chain_structure->senderratchetkeyprivate, private_key);
        if(result < 0) {
            goto complete;
        }
        chain_structure->has_senderratchetkeyprivate = 1;
    }

    if(chain->chain_key) {
        result = session_state_serialize_prepare_chain_chain_key(chain->chain_key, chain_structure);
        if(result < 0) {
            goto complete;
        }
    }

complete:
    return result;
}

static int session_state_serialize_prepare_receiver_chain(
        session_state_receiver_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure)
{
    int result = 0;

    if(chain->sender_ratchet_key) {
        result = ec_public_key_serialize_protobuf(&chain_structure->senderratchetkey, chain->sender_ratchet_key);
        if(result < 0) {
            goto complete;
        }
        chain_structure->has_senderratchetkey = 1;
    }

    if(chain->chain_key) {
        result = session_state_serialize_prepare_chain_chain_key(chain->chain_key, chain_structure);
        if(result < 0) {
            goto complete;
        }
    }

    if(chain->message_keys_head) {
        result = session_state_serialize_prepare_chain_message_keys_list(chain->message_keys_head, chain_structure);
        if(result < 0) {
            goto complete;
        }
    }

complete:
    return result;
}

static int session_state_serialize_prepare_chain_chain_key(
        ratchet_chain_key *chain_key,
        Textsecure__SessionStructure__Chain *chain_structure)
{
    int result = 0;
    chain_structure->chainkey = malloc(sizeof(Textsecure__SessionStructure__Chain__ChainKey));
    if(!chain_structure->chainkey) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    textsecure__session_structure__chain__chain_key__init(chain_structure->chainkey);

    chain_structure->chainkey->has_index = 1;
    chain_structure->chainkey->index = ratchet_chain_key_get_index(chain_key);

    result = ratchet_chain_key_get_key_protobuf(chain_key, &chain_structure->chainkey->key);
    if(result < 0) {
        goto complete;
    }
    chain_structure->chainkey->has_key = 1;

complete:
    return result;
}

static int session_state_serialize_prepare_chain_message_keys_list(
        message_keys_node *message_keys_head,
        Textsecure__SessionStructure__Chain *chain_structure)
{
    int result = 0;
    size_t count, i = 0;
    message_keys_node *cur_node;
    DL_COUNT(message_keys_head, cur_node, count);

    if(count == 0) {
        goto complete;
    }

    if(count > SIZE_MAX / sizeof(Textsecure__SessionStructure__Chain__MessageKey *)) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    chain_structure->messagekeys = malloc(sizeof(Textsecure__SessionStructure__Chain__MessageKey *) * count);
    if(!chain_structure->messagekeys) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    DL_FOREACH(message_keys_head, cur_node) {
        chain_structure->messagekeys[i] = malloc(sizeof(Textsecure__SessionStructure__Chain__MessageKey));
        if(!chain_structure->messagekeys[i]) {
            result = SG_ERR_NOMEM;
            break;
        }
        textsecure__session_structure__chain__message_key__init(chain_structure->messagekeys[i]);

        result = session_state_serialize_prepare_message_keys(&cur_node->message_key, chain_structure->messagekeys[i]);
        if(result < 0) {
            break;
        }
        i++;
    }
    chain_structure->n_messagekeys = i;
    if(result < 0) {
        goto complete;
    }

complete:
    return result;
}

static int session_state_serialize_prepare_message_keys(
        ratchet_message_keys *message_key,
        Textsecure__SessionStructure__Chain__MessageKey *message_key_structure)
{
    int result = 0;

    message_key_structure->has_index = 1;
    message_key_structure->index = message_key->counter;

    message_key_structure->cipherkey.data = malloc(sizeof(message_key->cipher_key));
    if(!message_key_structure->cipherkey.data) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memcpy(message_key_structure->cipherkey.data, message_key->cipher_key, sizeof(message_key->cipher_key));
    message_key_structure->cipherkey.len = sizeof(message_key->cipher_key);
    message_key_structure->has_cipherkey = 1;

    message_key_structure->mackey.data = malloc(sizeof(message_key->mac_key));
    if(!message_key_structure->mackey.data) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memcpy(message_key_structure->mackey.data, message_key->mac_key, sizeof(message_key->mac_key));
    message_key_structure->mackey.len = sizeof(message_key->mac_key);
    message_key_structure->has_mackey = 1;

    message_key_structure->iv.data = malloc(sizeof(message_key->iv));
    if(!message_key_structure->iv.data) {
        result = SG_ERR_NOMEM;
        goto complete;
    }
    memcpy(message_key_structure->iv.data, message_key->iv, sizeof(message_key->iv));
    message_key_structure->iv.len = sizeof(message_key->iv);
    message_key_structure->has_iv = 1;

complete:
    return result;
}

static void session_state_serialize_prepare_message_keys_free(
        Textsecure__SessionStructure__Chain__MessageKey *message_key_structure)
{
    if(message_key_structure->has_cipherkey) {
        free(message_key_structure->cipherkey.data);
    }
    if(message_key_structure->has_mackey) {
        free(message_key_structure->mackey.data);
    }
    if(message_key_structure->has_iv) {
        free(message_key_structure->iv.data);
    }
    free(message_key_structure);
}

static int session_state_serialize_prepare_pending_key_exchange(
        session_pending_key_exchange *exchange,
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure)
{
    int result = 0;

    exchange_structure->has_sequence = 1;
    exchange_structure->sequence = exchange->sequence;

    if(exchange->local_base_key) {
        ec_public_key *public_key = 0;
        ec_private_key *private_key = 0;

        public_key = ec_key_pair_get_public(exchange->local_base_key);
        result = ec_public_key_serialize_protobuf(&exchange_structure->localbasekey, public_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localbasekey = 1;

        private_key = ec_key_pair_get_private(exchange->local_base_key);
        result = ec_private_key_serialize_protobuf(&exchange_structure->localbasekeyprivate, private_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localbasekeyprivate = 1;
    }

    if(exchange->local_ratchet_key) {
        ec_public_key *public_key;
        ec_private_key *private_key;

        public_key = ec_key_pair_get_public(exchange->local_ratchet_key);
        result = ec_public_key_serialize_protobuf(&exchange_structure->localratchetkey, public_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localratchetkey = 1;

        private_key = ec_key_pair_get_private(exchange->local_ratchet_key);
        result = ec_private_key_serialize_protobuf(&exchange_structure->localratchetkeyprivate, private_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localratchetkeyprivate = 1;
    }

    if(exchange->local_identity_key) {
        ec_public_key *public_key;
        ec_private_key *private_key;

        public_key = ratchet_identity_key_pair_get_public(exchange->local_identity_key);
        result = ec_public_key_serialize_protobuf(&exchange_structure->localidentitykey, public_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localidentitykey = 1;

        private_key = ratchet_identity_key_pair_get_private(exchange->local_identity_key);
        result = ec_private_key_serialize_protobuf(&exchange_structure->localidentitykeyprivate, private_key);
        if(result < 0) {
            goto complete;
        }
        exchange_structure->has_localidentitykeyprivate = 1;
    }

complete:
    return result;
}

static int session_state_serialize_prepare_pending_pre_key(
        session_pending_pre_key *pre_key,
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure)
{
    int result = 0;

    if(pre_key->has_pre_key_id) {
        pre_key_structure->has_prekeyid = 1;
        pre_key_structure->prekeyid = pre_key->pre_key_id;
    }

    pre_key_structure->has_signedprekeyid = 1;
    pre_key_structure->signedprekeyid = (int32_t)pre_key->signed_pre_key_id;

    if(pre_key->base_key) {
        result = ec_public_key_serialize_protobuf(&pre_key_structure->basekey, pre_key->base_key);
        if(result < 0) {
            goto complete;
        }
        pre_key_structure->has_basekey = 1;
    }

complete:
    return result;
}

void session_state_serialize_prepare_free(Textsecure__SessionStructure *session_structure)
{
    assert(session_structure);

    if(session_structure->has_localidentitypublic) {
        free(session_structure->localidentitypublic.data);
    }

    if(session_structure->has_remoteidentitypublic) {
        free(session_structure->remoteidentitypublic.data);
    }

    if(session_structure->has_rootkey) {
        free(session_structure->rootkey.data);
    }

    if(session_structure->senderchain) {
        session_state_serialize_prepare_chain_free(session_structure->senderchain);
    }

    if(session_structure->receiverchains) {
        unsigned int i;
        for(i = 0; i < session_structure->n_receiverchains; i++) {
            if(session_structure->receiverchains[i]) {
                session_state_serialize_prepare_chain_free(session_structure->receiverchains[i]);
            }
        }
        free(session_structure->receiverchains);
    }

    if(session_structure->pendingkeyexchange) {
        session_state_serialize_prepare_pending_key_exchange_free(session_structure->pendingkeyexchange);
    }

    if(session_structure->pendingprekey) {
        session_state_serialize_prepare_pending_pre_key_free(session_structure->pendingprekey);
    }

    if(session_structure->has_alicebasekey) {
        free(session_structure->alicebasekey.data);
    }

    free(session_structure);
}

static void session_state_serialize_prepare_chain_free(
        Textsecure__SessionStructure__Chain *chain_structure)
{
    if(chain_structure->has_senderratchetkey) {
        free(chain_structure->senderratchetkey.data);
    }
    if(chain_structure->has_senderratchetkeyprivate) {
        free(chain_structure->senderratchetkeyprivate.data);
    }
    if(chain_structure->chainkey) {
        if(chain_structure->chainkey->has_key) {
            free(chain_structure->chainkey->key.data);
        }
        free(chain_structure->chainkey);
    }
    if(chain_structure->messagekeys) {
        unsigned int i;
        for(i = 0; i < chain_structure->n_messagekeys; i++) {
            if(chain_structure->messagekeys[i]) {
                session_state_serialize_prepare_message_keys_free(chain_structure->messagekeys[i]);
            }
        }
        free(chain_structure->messagekeys);
    }
    free(chain_structure);
}

static void session_state_serialize_prepare_pending_key_exchange_free(
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure)
{
    if(exchange_structure->has_localbasekey) {
        free(exchange_structure->localbasekey.data);
    }
    if(exchange_structure->has_localbasekeyprivate) {
        free(exchange_structure->localbasekeyprivate.data);
    }
    if(exchange_structure->has_localratchetkey) {
        free(exchange_structure->localratchetkey.data);
    }
    if(exchange_structure->has_localratchetkeyprivate) {
        free(exchange_structure->localratchetkeyprivate.data);
    }
    if(exchange_structure->has_localidentitykey) {
        free(exchange_structure->localidentitykey.data);
    }
    if(exchange_structure->has_localidentitykeyprivate) {
        free(exchange_structure->localidentitykeyprivate.data);
    }
    free(exchange_structure);
}

static void session_state_serialize_prepare_pending_pre_key_free(
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure)
{
    if(pre_key_structure->has_basekey) {
        free(pre_key_structure->basekey.data);
    }

    free(pre_key_structure);
}

int session_state_deserialize_protobuf(session_state **state, Textsecure__SessionStructure *session_structure, signal_context *global_context)
{
    int result = 0;
    session_state *result_state  = 0;

    result = session_state_create(&result_state, global_context);
    if(result < 0) {
        goto complete;
    }

    if(session_structure->has_sessionversion) {
        result_state->session_version = session_structure->sessionversion;
    }

    if(session_structure->has_localidentitypublic) {
        result = curve_decode_point(
                &result_state->local_identity_public,
                session_structure->localidentitypublic.data,
                session_structure->localidentitypublic.len,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

    if(session_structure->has_remoteidentitypublic) {
        result = curve_decode_point(
                &result_state->remote_identity_public,
                session_structure->remoteidentitypublic.data,
                session_structure->remoteidentitypublic.len,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

    if(session_structure->has_rootkey) {
        hkdf_context *kdf = 0;
        result = hkdf_create(&kdf, (int)result_state->session_version, global_context);
        if(result < 0) {
            goto complete;
        }

        result = ratchet_root_key_create(
                &result_state->root_key, kdf,
                session_structure->rootkey.data,
                session_structure->rootkey.len, global_context);
        SIGNAL_UNREF(kdf);
        if(result < 0) {
            goto complete;
        }
    }

    if(session_structure->has_previouscounter) {
        result_state->previous_counter = session_structure->previouscounter;
    }

    if(session_structure->senderchain) {
        session_state_deserialize_protobuf_sender_chain(
                result_state->session_version,
                &result_state->sender_chain, session_structure->senderchain,
                global_context);
        if(result < 0) {
            goto complete;
        }
        result_state->has_sender_chain = 1;
    }

    if(session_structure->n_receiverchains > 0) {
        unsigned int i;
        for(i = 0; i < session_structure->n_receiverchains; i++) {
            session_state_receiver_chain *node = malloc(sizeof(session_state_receiver_chain));
            if(!node) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
            memset(node, 0, sizeof(session_state_receiver_chain));

            result = session_state_deserialize_protobuf_receiver_chain(
                    result_state->session_version,
                    node, session_structure->receiverchains[i],
                    global_context);
            if(result < 0) {
                free(node);
                goto complete;
            }

            DL_APPEND(result_state->receiver_chain_head, node);
        }
    }

    if(session_structure->pendingkeyexchange) {
        result = session_state_deserialize_protobuf_pending_key_exchange(
                &result_state->pending_key_exchange,
                session_structure->pendingkeyexchange, global_context);
        if(result < 0) {
            goto complete;
        }
        result_state->has_pending_key_exchange = 1;
    }

    if(session_structure->pendingprekey) {
        result = session_state_deserialize_protobuf_pending_pre_key(
                &result_state->pending_pre_key,
                session_structure->pendingprekey, global_context);
        if(result < 0) {
            goto complete;
        }
        result_state->has_pending_pre_key = 1;
    }

    if(session_structure->has_remoteregistrationid) {
        result_state->remote_registration_id = session_structure->remoteregistrationid;
    }

    if(session_structure->has_localregistrationid) {
        result_state->local_registration_id = session_structure->localregistrationid;
    }

    if(session_structure->has_needsrefresh) {
        result_state->needs_refresh = session_structure->needsrefresh;
    }

    if(session_structure->has_alicebasekey) {
        result = curve_decode_point(
                &result_state->alice_base_key,
                session_structure->alicebasekey.data,
                session_structure->alicebasekey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

complete:
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

static int session_state_deserialize_protobuf_pending_key_exchange(
        session_pending_key_exchange *result_exchange,
        Textsecure__SessionStructure__PendingKeyExchange *exchange_structure,
        signal_context *global_context)
{
    int result = 0;

    ec_key_pair *local_base_key = 0;
    ec_public_key *local_base_key_public = 0;
    ec_private_key *local_base_key_private = 0;

    ec_key_pair *local_ratchet_key = 0;
    ec_public_key *local_ratchet_key_public = 0;
    ec_private_key *local_ratchet_key_private = 0;

    ratchet_identity_key_pair *local_identity_key = 0;
    ec_public_key *local_identity_key_public = 0;
    ec_private_key *local_identity_key_private = 0;

    if(exchange_structure->has_localbasekey && exchange_structure->has_localbasekeyprivate) {
        result = curve_decode_point(&local_base_key_public,
                exchange_structure->localbasekey.data,
                exchange_structure->localbasekey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = curve_decode_private_point(&local_base_key_private,
                exchange_structure->localbasekeyprivate.data,
                exchange_structure->localbasekeyprivate.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = ec_key_pair_create(&local_base_key,
                local_base_key_public, local_base_key_private);
        if(result < 0) {
            goto complete;
        }
    }

    if(exchange_structure->has_localratchetkey && exchange_structure->has_localratchetkeyprivate) {
        result = curve_decode_point(&local_ratchet_key_public,
                exchange_structure->localratchetkey.data,
                exchange_structure->localratchetkey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = curve_decode_private_point(&local_ratchet_key_private,
                exchange_structure->localratchetkeyprivate.data,
                exchange_structure->localratchetkeyprivate.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = ec_key_pair_create(&local_ratchet_key,
                local_ratchet_key_public, local_ratchet_key_private);
        if(result < 0) {
            goto complete;
        }
    }

    if(exchange_structure->has_localidentitykey && exchange_structure->has_localidentitykeyprivate) {
        result = curve_decode_point(&local_identity_key_public,
                exchange_structure->localidentitykey.data,
                exchange_structure->localidentitykey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = curve_decode_private_point(&local_identity_key_private,
                exchange_structure->localidentitykeyprivate.data,
                exchange_structure->localidentitykeyprivate.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = ratchet_identity_key_pair_create(&local_identity_key,
                local_identity_key_public,
                local_identity_key_private);
        if(result < 0) {
            goto complete;
        }
    }

    result_exchange->sequence = exchange_structure->sequence;
    result_exchange->local_base_key = local_base_key;
    result_exchange->local_ratchet_key = local_ratchet_key;
    result_exchange->local_identity_key = local_identity_key;

complete:
    SIGNAL_UNREF(local_base_key_public);
    SIGNAL_UNREF(local_base_key_private);
    SIGNAL_UNREF(local_ratchet_key_public);
    SIGNAL_UNREF(local_ratchet_key_private);
    SIGNAL_UNREF(local_identity_key_public);
    SIGNAL_UNREF(local_identity_key_private);

    if(result < 0) {
        SIGNAL_UNREF(local_base_key);
        SIGNAL_UNREF(local_ratchet_key);
        SIGNAL_UNREF(local_identity_key);
    }

    return result;
}

static int session_state_deserialize_protobuf_pending_pre_key(
        session_pending_pre_key *result_pre_key,
        Textsecure__SessionStructure__PendingPreKey *pre_key_structure,
        signal_context *global_context)
{
    int result = 0;

    if(pre_key_structure->has_basekey) {
        ec_public_key *base_key = 0;
        result = curve_decode_point(&base_key,
                pre_key_structure->basekey.data,
                pre_key_structure->basekey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }
        result_pre_key->base_key = base_key;
    }

    if(pre_key_structure->has_prekeyid) {
        result_pre_key->has_pre_key_id = 1;
        result_pre_key->pre_key_id = pre_key_structure->prekeyid;
    }

    if(pre_key_structure->has_signedprekeyid) {
        result_pre_key->signed_pre_key_id = (uint32_t)pre_key_structure->signedprekeyid;
    }

complete:
    return result;
}

static int session_state_deserialize_protobuf_sender_chain(
        uint32_t session_version,
        session_state_sender_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure,
        signal_context *global_context)
{
    int result = 0;
    hkdf_context *kdf = 0;
    ec_key_pair *sender_ratchet_key_pair = 0;
    ec_public_key *sender_ratchet_key_public = 0;
    ec_private_key *sender_ratchet_key_private = 0;
    ratchet_chain_key *sender_chain_key = 0;

    if(chain_structure->has_senderratchetkey && chain_structure->has_senderratchetkeyprivate) {
        result = curve_decode_point(&sender_ratchet_key_public,
                chain_structure->senderratchetkey.data,
                chain_structure->senderratchetkey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = curve_decode_private_point(&sender_ratchet_key_private,
                chain_structure->senderratchetkeyprivate.data,
                chain_structure->senderratchetkeyprivate.len,
                global_context);
        if(result < 0) {
            goto complete;
        }

        result = ec_key_pair_create(&sender_ratchet_key_pair,
                sender_ratchet_key_public, sender_ratchet_key_private);
        if(result < 0) {
            goto complete;
        }
    }

    if(chain_structure->chainkey && chain_structure->chainkey->has_key && chain_structure->chainkey->has_index) {
        result = hkdf_create(&kdf, (int)session_version, global_context);
        if(result < 0) {
            goto complete;
        }

        result = ratchet_chain_key_create(
                &sender_chain_key, kdf,
                chain_structure->chainkey->key.data,
                chain_structure->chainkey->key.len,
                chain_structure->chainkey->index,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

    chain->sender_ratchet_key_pair = sender_ratchet_key_pair;
    chain->chain_key = sender_chain_key;

complete:
    SIGNAL_UNREF(kdf);
    SIGNAL_UNREF(sender_ratchet_key_public);
    SIGNAL_UNREF(sender_ratchet_key_private);
    if(result < 0) {
        SIGNAL_UNREF(sender_ratchet_key_pair);
        SIGNAL_UNREF(sender_chain_key);
    }
    return result;
}

static int session_state_deserialize_protobuf_receiver_chain(
        uint32_t session_version,
        session_state_receiver_chain *chain,
        Textsecure__SessionStructure__Chain *chain_structure,
        signal_context *global_context)
{
    int result = 0;

    hkdf_context *kdf = 0;
    ec_public_key *sender_ratchet_key = 0;
    ratchet_chain_key *chain_key = 0;
    message_keys_node *message_keys_head = 0;

    if(chain_structure->has_senderratchetkey) {
        result = curve_decode_point(&sender_ratchet_key,
                chain_structure->senderratchetkey.data,
                chain_structure->senderratchetkey.len,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

    if(chain_structure->chainkey && chain_structure->chainkey->has_key && chain_structure->chainkey->has_index) {
        result = hkdf_create(&kdf, (int)session_version, global_context);
        if(result < 0) {
            goto complete;
        }

        result = ratchet_chain_key_create(
                &chain_key, kdf,
                chain_structure->chainkey->key.data,
                chain_structure->chainkey->key.len,
                chain_structure->chainkey->index,
                global_context);
        if(result < 0) {
            goto complete;
        }
    }

    if(chain_structure->n_messagekeys > 0) {
        unsigned int i;
        for(i = 0; i < chain_structure->n_messagekeys; i++) {
            Textsecure__SessionStructure__Chain__MessageKey *key_structure =
                    chain_structure->messagekeys[i];

            message_keys_node *node = malloc(sizeof(message_keys_node));
            if(!node) {
                result = SG_ERR_NOMEM;
                goto complete;
            }
            memset(node, 0, sizeof(message_keys_node));

            if(key_structure->has_index) {
                node->message_key.counter = key_structure->index;
            }
            if(key_structure->has_cipherkey && key_structure->cipherkey.len == sizeof(node->message_key.cipher_key)) {
                memcpy(node->message_key.cipher_key, key_structure->cipherkey.data, key_structure->cipherkey.len);
            }
            if(key_structure->has_mackey && key_structure->mackey.len == sizeof(node->message_key.mac_key)) {
                memcpy(node->message_key.mac_key, key_structure->mackey.data, key_structure->mackey.len);
            }
            if(key_structure->has_iv && key_structure->iv.len == sizeof(node->message_key.iv)) {
                memcpy(node->message_key.iv, key_structure->iv.data, key_structure->iv.len);
            }

            DL_APPEND(message_keys_head, node);
        }
    }

    chain->sender_ratchet_key = sender_ratchet_key;
    chain->chain_key = chain_key;
    chain->message_keys_head = message_keys_head;

complete:
    SIGNAL_UNREF(kdf);
    if(result < 0) {
        SIGNAL_UNREF(sender_ratchet_key);
        SIGNAL_UNREF(chain_key);
        if(message_keys_head) {
            message_keys_node *cur_node;
            message_keys_node *tmp_node;
            DL_FOREACH_SAFE(message_keys_head, cur_node, tmp_node) {
                DL_DELETE(message_keys_head, cur_node);
                signal_explicit_bzero(&cur_node->message_key, sizeof(ratchet_message_keys));
                free(cur_node);
            }
        }
    }
    return result;
}

int session_state_copy(session_state **state, session_state *other_state, signal_context *global_context)
{
    int result = 0;
    signal_buffer *buffer = 0;
    size_t len = 0;
    uint8_t *data = 0;

    assert(other_state);
    assert(global_context);

    result = session_state_serialize(&buffer, other_state);
    if(result < 0) {
        goto complete;
    }

    data = signal_buffer_data(buffer);
    len = signal_buffer_len(buffer);

    result = session_state_deserialize(state, data, len, global_context);
    if(result < 0) {
        goto complete;
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    return result;
}

void session_state_set_session_version(session_state *state, uint32_t version)
{
    assert(state);
    state->session_version = version;
}

uint32_t session_state_get_session_version(const session_state *state)
{
    assert(state);
    return state->session_version;
}

void session_state_set_local_identity_key(session_state *state, ec_public_key *identity_key)
{
    assert(state);
    assert(identity_key);
    if(state->local_identity_public) {
        SIGNAL_UNREF(state->local_identity_public);
    }
    SIGNAL_REF(identity_key);
    state->local_identity_public = identity_key;
}

ec_public_key *session_state_get_local_identity_key(const session_state *state)
{
    assert(state);
    return state->local_identity_public;
}

void session_state_set_remote_identity_key(session_state *state, ec_public_key *identity_key)
{
    assert(state);
    assert(identity_key);
    if(state->remote_identity_public) {
        SIGNAL_UNREF(state->remote_identity_public);
    }
    SIGNAL_REF(identity_key);
    state->remote_identity_public = identity_key;
}

ec_public_key *session_state_get_remote_identity_key(const session_state *state)
{
    assert(state);
    return state->remote_identity_public;
}

void session_state_set_root_key(session_state *state, ratchet_root_key *root_key)
{
    assert(state);
    assert(root_key);
    if(state->root_key) {
        SIGNAL_UNREF(state->root_key);
    }
    SIGNAL_REF(root_key);
    state->root_key = root_key;
}

ratchet_root_key *session_state_get_root_key(const session_state *state)
{
    assert(state);
    return state->root_key;
}

void session_state_set_previous_counter(session_state *state, uint32_t counter)
{
    assert(state);
    state->previous_counter = counter;
}

uint32_t session_state_get_previous_counter(const session_state *state)
{
    assert(state);
    return state->previous_counter;
}

void session_state_set_sender_chain(session_state *state, ec_key_pair *sender_ratchet_key_pair, ratchet_chain_key *chain_key)
{
    assert(state);
    assert(sender_ratchet_key_pair);
    assert(chain_key);

    state->has_sender_chain = 1;

    if(state->sender_chain.sender_ratchet_key_pair) {
        SIGNAL_UNREF(state->sender_chain.sender_ratchet_key_pair);
    }
    SIGNAL_REF(sender_ratchet_key_pair);
    state->sender_chain.sender_ratchet_key_pair = sender_ratchet_key_pair;

    if(state->sender_chain.chain_key) {
        SIGNAL_UNREF(state->sender_chain.chain_key);
    }
    SIGNAL_REF(chain_key);
    state->sender_chain.chain_key = chain_key;
}

ec_public_key *session_state_get_sender_ratchet_key(const session_state *state)
{
    assert(state);
    if(state->sender_chain.sender_ratchet_key_pair) {
        return ec_key_pair_get_public(state->sender_chain.sender_ratchet_key_pair);
    }
    else {
        return 0;
    }
}

ec_key_pair *session_state_get_sender_ratchet_key_pair(const session_state *state)
{
    assert(state);
    return state->sender_chain.sender_ratchet_key_pair;
}

ratchet_chain_key *session_state_get_sender_chain_key(const session_state *state)
{
    assert(state);
    return state->sender_chain.chain_key;
}

int session_state_set_sender_chain_key(session_state *state, ratchet_chain_key *chain_key)
{
    assert(state);
    if(state->has_sender_chain) {
        if(state->sender_chain.chain_key) {
            SIGNAL_UNREF(state->sender_chain.chain_key);
        }
        SIGNAL_REF(chain_key);
        state->sender_chain.chain_key = chain_key;
        return 0;
    }
    else {
        return SG_ERR_UNKNOWN;
    }
}

int session_state_has_sender_chain(const session_state *state)
{
    assert(state);
    return state->has_sender_chain;
}

int session_state_has_message_keys(session_state *state, ec_public_key *sender_ephemeral, uint32_t counter)
{
    session_state_receiver_chain *chain = 0;
    message_keys_node *cur_node = 0;

    assert(state);
    assert(sender_ephemeral);

    chain = session_state_find_receiver_chain(state, sender_ephemeral);
    if(!chain) {
        return 0;
    }

    DL_FOREACH(chain->message_keys_head, cur_node) {
        if(cur_node->message_key.counter == counter) {
            return 1;
        }
    }

    return 0;
}

int session_state_remove_message_keys(session_state *state,
        ratchet_message_keys *message_keys_result,
        ec_public_key *sender_ephemeral, uint32_t counter)
{
    session_state_receiver_chain *chain = 0;
    message_keys_node *cur_node = 0;
    message_keys_node *tmp_node = 0;

    assert(state);
    assert(message_keys_result);
    assert(sender_ephemeral);

    chain = session_state_find_receiver_chain(state, sender_ephemeral);
    if(!chain) {
        return 0;
    }

    DL_FOREACH_SAFE(chain->message_keys_head, cur_node, tmp_node) {
        if(cur_node->message_key.counter == counter) {
            memcpy(message_keys_result, &(cur_node->message_key), sizeof(ratchet_message_keys));
            DL_DELETE(chain->message_keys_head, cur_node);
            signal_explicit_bzero(&cur_node->message_key, sizeof(ratchet_message_keys));
            free(cur_node);
            return 1;
        }
    }

    return 0;
}

int session_state_set_message_keys(session_state *state,
        ec_public_key *sender_ephemeral, ratchet_message_keys *message_keys)
{
    session_state_receiver_chain *chain = 0;
    message_keys_node *node = 0;
    int count;

    assert(state);
    assert(sender_ephemeral);
    assert(message_keys);

    chain = session_state_find_receiver_chain(state, sender_ephemeral);
    if(!chain) {
        return 0;
    }

    node = malloc(sizeof(message_keys_node));
    if(!node) {
        return SG_ERR_NOMEM;
    }
    memcpy(&(node->message_key), message_keys, sizeof(ratchet_message_keys));
    node->prev = 0;
    node->next = 0;

    DL_APPEND(chain->message_keys_head, node);

    DL_COUNT(chain->message_keys_head, node, count);
    while(count > MAX_MESSAGE_KEYS) {
        node = chain->message_keys_head;
        DL_DELETE(chain->message_keys_head, node);
        signal_explicit_bzero(&node->message_key, sizeof(ratchet_message_keys));
        free(node);
        --count;
    }

    return 0;
}

int session_state_add_receiver_chain(session_state *state, ec_public_key *sender_ratchet_key, ratchet_chain_key *chain_key)
{
    session_state_receiver_chain *node;
    int count;

    assert(state);
    assert(sender_ratchet_key);
    assert(chain_key);

    node = malloc(sizeof(session_state_receiver_chain));
    if(!node) {
        return SG_ERR_NOMEM;
    }
    memset(node, 0, sizeof(session_state_receiver_chain));

    SIGNAL_REF(sender_ratchet_key);
    node->sender_ratchet_key = sender_ratchet_key;
    SIGNAL_REF(chain_key);
    node->chain_key = chain_key;

    DL_APPEND(state->receiver_chain_head, node);

    DL_COUNT(state->receiver_chain_head, node, count);
    while(count > 5) {
        node = state->receiver_chain_head;
        DL_DELETE(state->receiver_chain_head, node);
        session_state_free_receiver_chain_node(node);
        --count;
    }

    return 0;
}

int session_state_set_receiver_chain_key(session_state *state, ec_public_key *sender_ephemeral, ratchet_chain_key *chain_key)
{
    int result = 0;
    session_state_receiver_chain *node;

    assert(state);
    assert(sender_ephemeral);
    assert(chain_key);

    node = session_state_find_receiver_chain(state, sender_ephemeral);
    if(!node) {
        signal_log(state->global_context, SG_LOG_WARNING, "Couldn't find receiver chain to set chain key on");
        result = SG_ERR_UNKNOWN;
        goto complete;
    }

    SIGNAL_UNREF(node->chain_key);
    SIGNAL_REF(chain_key);
    node->chain_key = chain_key;

complete:
    return result;
}

static session_state_receiver_chain *session_state_find_receiver_chain(const session_state *state, const ec_public_key *sender_ephemeral)
{
    session_state_receiver_chain *result = 0;

    session_state_receiver_chain *cur_node;
    DL_FOREACH(state->receiver_chain_head, cur_node) {
        if(ec_public_key_compare(cur_node->sender_ratchet_key, sender_ephemeral) == 0) {
            result = cur_node;
            break;
        }
    }

    return result;
}

ratchet_chain_key *session_state_get_receiver_chain_key(session_state *state, ec_public_key *sender_ephemeral)
{
    ratchet_chain_key *result = 0;
    session_state_receiver_chain *node = session_state_find_receiver_chain(state, sender_ephemeral);

    if(node) {
        result = node->chain_key;
    }

    return result;
}

void session_state_set_pending_key_exchange(session_state *state,
        uint32_t sequence,
        ec_key_pair *our_base_key, ec_key_pair *our_ratchet_key,
        ratchet_identity_key_pair *our_identity_key)
{
    assert(state);
    assert(our_base_key);
    assert(our_ratchet_key);
    assert(our_identity_key);

    if(state->pending_key_exchange.local_base_key) {
        SIGNAL_UNREF(state->pending_key_exchange.local_base_key);
        state->pending_key_exchange.local_base_key = 0;
    }
    if(state->pending_key_exchange.local_ratchet_key) {
        SIGNAL_UNREF(state->pending_key_exchange.local_ratchet_key);
        state->pending_key_exchange.local_ratchet_key = 0;
    }
    if(state->pending_key_exchange.local_identity_key) {
        SIGNAL_UNREF(state->pending_key_exchange.local_identity_key);
        state->pending_key_exchange.local_identity_key = 0;
    }

    SIGNAL_REF(our_base_key);
    SIGNAL_REF(our_ratchet_key);
    SIGNAL_REF(our_identity_key);

    state->has_pending_key_exchange = 1;
    state->pending_key_exchange.sequence = sequence;
    state->pending_key_exchange.local_base_key = our_base_key;
    state->pending_key_exchange.local_ratchet_key = our_ratchet_key;
    state->pending_key_exchange.local_identity_key = our_identity_key;
}

uint32_t session_state_get_pending_key_exchange_sequence(session_state *state)
{
    assert(state);
    if(state->has_pending_key_exchange) {
        return state->pending_key_exchange.sequence;
    }
    else {
        return 0;
    }
}

ec_key_pair *session_state_get_pending_key_exchange_base_key(const session_state *state)
{
    assert(state);
    if(state->has_pending_key_exchange) {
        return state->pending_key_exchange.local_base_key;
    }
    else {
        return 0;
    }
}

ec_key_pair *session_state_get_pending_key_exchange_ratchet_key(const session_state *state)
{
    assert(state);
    if(state->has_pending_key_exchange) {
        return state->pending_key_exchange.local_ratchet_key;
    }
    else {
        return 0;
    }
}

ratchet_identity_key_pair *session_state_get_pending_key_exchange_identity_key(const session_state *state)
{
    assert(state);
    if(state->has_pending_key_exchange) {
        return state->pending_key_exchange.local_identity_key;
    }
    else {
        return 0;
    }
}

int session_state_has_pending_key_exchange(const session_state *state)
{
    assert(state);
    return state->has_pending_key_exchange;
}

void session_state_set_unacknowledged_pre_key_message(session_state *state,
        const uint32_t *pre_key_id, uint32_t signed_pre_key_id, ec_public_key *base_key)
{
    assert(state);
    assert(base_key);

    if(state->pending_pre_key.base_key) {
        SIGNAL_UNREF(state->pending_pre_key.base_key);
        state->pending_pre_key.base_key = 0;
    }

    SIGNAL_REF(base_key);

    state->has_pending_pre_key = 1;
    if(pre_key_id) {
        state->pending_pre_key.has_pre_key_id = 1;
        state->pending_pre_key.pre_key_id = *pre_key_id;
    }
    else {
        state->pending_pre_key.has_pre_key_id = 0;
        state->pending_pre_key.pre_key_id = 0;
    }
    state->pending_pre_key.signed_pre_key_id = signed_pre_key_id;
    state->pending_pre_key.base_key = base_key;
}

int session_state_unacknowledged_pre_key_message_has_pre_key_id(const session_state *state)
{
    assert(state);
    return state->pending_pre_key.has_pre_key_id;
}

uint32_t session_state_unacknowledged_pre_key_message_get_pre_key_id(const session_state *state)
{
    assert(state);
    assert(state->pending_pre_key.has_pre_key_id);
    return state->pending_pre_key.pre_key_id;
}

uint32_t session_state_unacknowledged_pre_key_message_get_signed_pre_key_id(const session_state *state)
{
    assert(state);
    return state->pending_pre_key.signed_pre_key_id;
}

ec_public_key *session_state_unacknowledged_pre_key_message_get_base_key(const session_state *state)
{
    assert(state);
    return state->pending_pre_key.base_key;
}

int session_state_has_unacknowledged_pre_key_message(const session_state *state)
{
    assert(state);
    return state->has_pending_pre_key;
}

void session_state_clear_unacknowledged_pre_key_message(session_state *state)
{
    assert(state);
    if(state->pending_pre_key.base_key) {
        SIGNAL_UNREF(state->pending_pre_key.base_key);
    }
    memset(&state->pending_pre_key, 0, sizeof(state->pending_pre_key));
    state->has_pending_pre_key = 0;
}

void session_state_set_remote_registration_id(session_state *state, uint32_t id)
{
    assert(state);
    state->remote_registration_id = id;
}

uint32_t session_state_get_remote_registration_id(const session_state *state)
{
    assert(state);
    return state->remote_registration_id;
}

void session_state_set_local_registration_id(session_state *state, uint32_t id)
{
    assert(state);
    state->local_registration_id = id;
}

uint32_t session_state_get_local_registration_id(const session_state *state)
{
    assert(state);
    return state->local_registration_id;
}

void session_state_set_needs_refresh(session_state *state, int value)
{
    assert(state);
    assert(value == 0 || value == 1);
    state->needs_refresh = value;
}

int session_state_get_needs_refresh(const session_state *state)
{
    assert(state);
    return state->needs_refresh;
}

void session_state_set_alice_base_key(session_state *state, ec_public_key *key)
{
    assert(state);
    assert(key);

    if(state->alice_base_key) {
        SIGNAL_UNREF(state->alice_base_key);
    }
    SIGNAL_REF(key);
    state->alice_base_key = key;
}

ec_public_key *session_state_get_alice_base_key(const session_state *state)
{
    assert(state);
    return state->alice_base_key;
}

static void session_state_free_sender_chain(session_state *state)
{
    if(state->sender_chain.sender_ratchet_key_pair) {
        SIGNAL_UNREF(state->sender_chain.sender_ratchet_key_pair);
        state->sender_chain.sender_ratchet_key_pair = 0;
    }
    if(state->sender_chain.chain_key) {
        SIGNAL_UNREF(state->sender_chain.chain_key);
        state->sender_chain.chain_key = 0;
    }
}

static void session_state_free_receiver_chain_node(session_state_receiver_chain *node)
{
    if(node->sender_ratchet_key) {
        SIGNAL_UNREF(node->sender_ratchet_key);
    }
    if(node->chain_key) {
        SIGNAL_UNREF(node->chain_key);
    }

    if(node->message_keys_head) {
        message_keys_node *cur_node;
        message_keys_node *tmp_node;
        DL_FOREACH_SAFE(node->message_keys_head, cur_node, tmp_node) {
            DL_DELETE(node->message_keys_head, cur_node);
            signal_explicit_bzero(&cur_node->message_key, sizeof(ratchet_message_keys));
            free(cur_node);
        }
        node->message_keys_head = 0;
    }

    free(node);
}

static void session_state_free_receiver_chain(session_state *state)
{
    session_state_receiver_chain *cur_node;
    session_state_receiver_chain *tmp_node;
    DL_FOREACH_SAFE(state->receiver_chain_head, cur_node, tmp_node) {
        DL_DELETE(state->receiver_chain_head, cur_node);
        session_state_free_receiver_chain_node(cur_node);
    }
    state->receiver_chain_head = 0;
}

void session_state_destroy(signal_type_base *type)
{
    session_state *state = (session_state *)type;

    if(state->local_identity_public) {
        SIGNAL_UNREF(state->local_identity_public);
    }
    if(state->remote_identity_public) {
        SIGNAL_UNREF(state->remote_identity_public);
    }
    if(state->root_key) {
        SIGNAL_UNREF(state->root_key);
    }
    session_state_free_sender_chain(state);
    session_state_free_receiver_chain(state);
    if(state->has_pending_key_exchange) {
        if(state->pending_key_exchange.local_base_key) {
            SIGNAL_UNREF(state->pending_key_exchange.local_base_key);
        }
        if(state->pending_key_exchange.local_ratchet_key) {
            SIGNAL_UNREF(state->pending_key_exchange.local_ratchet_key);
        }
        if(state->pending_key_exchange.local_identity_key) {
            SIGNAL_UNREF(state->pending_key_exchange.local_identity_key);
        }
    }
    if(state->has_pending_pre_key) {
        if(state->pending_pre_key.base_key) {
            SIGNAL_UNREF(state->pending_pre_key.base_key);
        }
    }
    if(state->alice_base_key) {
        SIGNAL_UNREF(state->alice_base_key);
    }
    free(state);
}
