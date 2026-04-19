#include "session_record.h"

#include <stdlib.h>
#include <string.h>
#include <assert.h>

#include "session_state.h"
#include "utlist.h"
#include "LocalStorageProtocol.pb-c.h"
#include "signal_protocol_internal.h"

#define ARCHIVED_STATES_MAX_LENGTH 40

struct session_record_state_node
{
    session_state *state;
    struct session_record_state_node *prev, *next;
};

struct session_record
{
    signal_type_base base;
    session_state *state;
    session_record_state_node *previous_states_head;
    int is_fresh;
    signal_buffer *user_record;
    signal_context *global_context;
};

static void session_record_free_previous_states(session_record *record);

int session_record_create(session_record **record, session_state *state, signal_context *global_context)
{
    session_record *result = malloc(sizeof(session_record));
    if(!result) {
        return SG_ERR_NOMEM;
    }
    memset(result, 0, sizeof(session_record));
    SIGNAL_INIT(result, session_record_destroy);

    if(!state) {
        int ret = session_state_create(&result->state, global_context);
        if(ret < 0) {
            SIGNAL_UNREF(result);
            return ret;
        }
        result->is_fresh = 1;
    }
    else {
        SIGNAL_REF(state);
        result->state = state;
        result->is_fresh = 0;
    }
    result->global_context = global_context;

    *record = result;
    return 0;
}

int session_record_serialize(signal_buffer **buffer, const session_record *record)
{
    int result = 0;
    size_t result_size = 0;
    unsigned int i = 0;
    Textsecure__RecordStructure record_structure = TEXTSECURE__RECORD_STRUCTURE__INIT;
    session_record_state_node *cur_node = 0;
    signal_buffer *result_buf = 0;
    size_t len = 0;
    uint8_t *data = 0;

    if(!record) {
        result = SG_ERR_INVAL;
        goto complete;
    }

    if(record->state) {
        record_structure.currentsession = malloc(sizeof(Textsecure__SessionStructure));
        if(!record_structure.currentsession) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
        textsecure__session_structure__init(record_structure.currentsession);
        result = session_state_serialize_prepare(record->state, record_structure.currentsession);
        if(result < 0) {
            goto complete;
        }
    }

    if(record->previous_states_head) {
        size_t count;
        DL_COUNT(record->previous_states_head, cur_node, count);

        if(count > SIZE_MAX / sizeof(Textsecure__SessionStructure *)) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        record_structure.previoussessions = malloc(sizeof(Textsecure__SessionStructure *) * count);
        if(!record_structure.previoussessions) {
            result = SG_ERR_NOMEM;
            goto complete;
        }

        i = 0;
        DL_FOREACH(record->previous_states_head, cur_node) {
            record_structure.previoussessions[i] = malloc(sizeof(Textsecure__SessionStructure));
            if(!record_structure.previoussessions[i]) {
                result = SG_ERR_NOMEM;
                break;
            }
            textsecure__session_structure__init(record_structure.previoussessions[i]);
            result = session_state_serialize_prepare(cur_node->state, record_structure.previoussessions[i]);
            if(result < 0) {
                break;
            }
            i++;
        }
        record_structure.n_previoussessions = i;
        if(result < 0) {
            goto complete;
        }
    }

    len = textsecure__record_structure__get_packed_size(&record_structure);

    result_buf = signal_buffer_alloc(len);
    if(!result_buf) {
        result = SG_ERR_NOMEM;
        goto complete;
    }

    data = signal_buffer_data(result_buf);
    result_size = textsecure__record_structure__pack(&record_structure, data);
    if(result_size != len) {
        signal_buffer_free(result_buf);
        result = SG_ERR_INVALID_PROTO_BUF;
        result_buf = 0;
        goto complete;
    }

complete:
    if(record_structure.currentsession) {
        session_state_serialize_prepare_free(record_structure.currentsession);
    }
    if(record_structure.previoussessions) {
        for(i = 0; i < record_structure.n_previoussessions; i++) {
            if(record_structure.previoussessions[i]) {
                session_state_serialize_prepare_free(record_structure.previoussessions[i]);
            }
        }
        free(record_structure.previoussessions);
    }

    if(result >= 0) {
        *buffer = result_buf;
    }
    return result;
}

int session_record_deserialize(session_record **record, const uint8_t *data, size_t len, signal_context *global_context)
{
    int result = 0;
    session_record *result_record = 0;
    session_state *current_state = 0;
    session_record_state_node *previous_states_head = 0;
    Textsecure__RecordStructure *record_structure = 0;

    record_structure = textsecure__record_structure__unpack(0, len, data);
    if(!record_structure) {
        result = SG_ERR_INVALID_PROTO_BUF;
        goto complete;
    }

    if(record_structure->currentsession) {
        result = session_state_deserialize_protobuf(&current_state, record_structure->currentsession, global_context);
        if(result < 0) {
            goto complete;
        }
    }

    result = session_record_create(&result_record, current_state, global_context);
    if(result < 0) {
        goto complete;
    }
    SIGNAL_UNREF(current_state);
    current_state = 0;
    result_record->is_fresh = 0;

    if(record_structure->n_previoussessions > 0) {
        unsigned int i;
        for(i = 0; i < record_structure->n_previoussessions; i++) {
            Textsecure__SessionStructure *session_structure =
                    record_structure->previoussessions[i];

            session_record_state_node *node = malloc(sizeof(session_record_state_node));
            if(!node) {
                result = SG_ERR_NOMEM;
                goto complete;
            }

            result = session_state_deserialize_protobuf(&node->state, session_structure, global_context);
            if(result < 0) {
                free(node);
                goto complete;
            }

            DL_APPEND(previous_states_head, node);
        }
    }
    result_record->previous_states_head = previous_states_head;
    previous_states_head = 0;

complete:
    if(record_structure) {
        textsecure__record_structure__free_unpacked(record_structure, 0);
    }
    if(current_state) {
        SIGNAL_UNREF(current_state);
    }
    if(previous_states_head) {
        session_record_state_node *cur_node;
        session_record_state_node *tmp_node;
        DL_FOREACH_SAFE(previous_states_head, cur_node, tmp_node) {
            DL_DELETE(previous_states_head, cur_node);
            free(cur_node);
        }
    }
    if(result_record) {
        if(result < 0) {
            SIGNAL_UNREF(result_record);
        }
        else {
            *record = result_record;
        }
    }

    return result;
}

int session_record_copy(session_record **record, session_record *other_record, signal_context *global_context)
{
    int result = 0;
    session_record *result_record = 0;
    signal_buffer *buffer = 0;
    size_t len = 0;
    uint8_t *data = 0;

    assert(other_record);
    assert(global_context);

    result = session_record_serialize(&buffer, other_record);
    if(result < 0) {
        goto complete;
    }

    data = signal_buffer_data(buffer);
    len = signal_buffer_len(buffer);

    result = session_record_deserialize(&result_record, data, len, global_context);
    if(result < 0) {
        goto complete;
    }
    if(other_record->user_record) {
        result_record->user_record = signal_buffer_copy(other_record->user_record);
        if(!result_record->user_record) {
            result = SG_ERR_NOMEM;
            goto complete;
        }
    }

complete:
    if(buffer) {
        signal_buffer_free(buffer);
    }
    if(result >= 0) {
        *record = result_record;
    }
    else {
        SIGNAL_UNREF(result_record);
    }
    return result;
}

int session_record_has_session_state(session_record *record, uint32_t version, const ec_public_key *alice_base_key)
{
    session_record_state_node *cur_node = 0;

    assert(record);
    assert(record->state);

    if(session_state_get_session_version(record->state) == version &&
            ec_public_key_compare(
                    session_state_get_alice_base_key(record->state),
                    alice_base_key) == 0) {
        return 1;
    }

    DL_FOREACH(record->previous_states_head, cur_node) {
        if(session_state_get_session_version(cur_node->state) == version &&
                ec_public_key_compare(
                        session_state_get_alice_base_key(cur_node->state),
                        alice_base_key) == 0) {
            return 1;
        }
    }

    return 0;
}

session_state *session_record_get_state(session_record *record)
{
    return record->state;
}

void session_record_set_state(session_record *record, session_state *state)
{
    assert(record);
    assert(state);
    if(record->state) {
        SIGNAL_UNREF(record->state);
    }
    SIGNAL_REF(state);
    record->state = state;
}

session_record_state_node *session_record_get_previous_states_head(const session_record *record)
{
    assert(record);
    return record->previous_states_head;
}

session_state *session_record_get_previous_states_element(const session_record_state_node *node)
{
    assert(node);
    return node->state;
}

session_record_state_node *session_record_get_previous_states_next(const session_record_state_node *node)
{
    assert(node);
    return node->next;
}

session_record_state_node *session_record_get_previous_states_remove(session_record *record, session_record_state_node *node)
{
    session_record_state_node *next_node = 0;

    assert(record);
    assert(node);

    next_node = node->next;
    DL_DELETE(record->previous_states_head, node);
    SIGNAL_UNREF(node->state);
    free(node);
    return next_node;
}

int session_record_is_fresh(session_record *record)
{
    assert(record);
    return record->is_fresh;
}

int session_record_archive_current_state(session_record *record)
{
    int result = 0;
    session_state *new_state = 0;

    assert(record);

    result = session_state_create(&new_state, record->global_context);
    if(result < 0) {
        goto complete;
    }

    result = session_record_promote_state(record, new_state);

complete:
    SIGNAL_UNREF(new_state);
    return result;
}

int session_record_promote_state(session_record *record, session_state *promoted_state)
{
    int count = 0;
    session_record_state_node *cur_node = 0;
    session_record_state_node *tmp_node = 0;

    assert(record);
    assert(promoted_state);

    // Move the previously current state to the list of previous states
    if(record->state) {
        session_record_state_node *node = malloc(sizeof(session_record_state_node));
        if(!node) {
            return SG_ERR_NOMEM;
        }

        node->state = record->state;
        DL_PREPEND(record->previous_states_head, node);
        record->state = 0;
    }

    // Make the promoted state the current state
    SIGNAL_REF(promoted_state);
    record->state = promoted_state;

    // Remove any previous nodes beyond the maximum length
    DL_FOREACH_SAFE(record->previous_states_head, cur_node, tmp_node) {
        count++;
        if(count > ARCHIVED_STATES_MAX_LENGTH) {
            DL_DELETE(record->previous_states_head, cur_node);
            if(cur_node->state) {
                SIGNAL_UNREF(cur_node->state);
            }
            free(cur_node);
        }
    }

    return 0;
}

static void session_record_free_previous_states(session_record *record)
{
    session_record_state_node *cur_node;
    session_record_state_node *tmp_node;
    DL_FOREACH_SAFE(record->previous_states_head, cur_node, tmp_node) {
        DL_DELETE(record->previous_states_head, cur_node);
        if(cur_node->state) {
            SIGNAL_UNREF(cur_node->state);
        }
        free(cur_node);
    }
    record->previous_states_head = 0;
}

signal_buffer *session_record_get_user_record(const session_record *record)
{
    assert(record);
    return record->user_record;
}

void session_record_set_user_record(session_record *record, signal_buffer *user_record)
{
    assert(record);
    if(record->user_record) {
        signal_buffer_free(record->user_record);
    }
    record->user_record = user_record;
}

void session_record_destroy(signal_type_base *type)
{
    session_record *record = (session_record *)type;

    if(record->state) {
        SIGNAL_UNREF(record->state);
    }
    session_record_free_previous_states(record);

    if(record->user_record) {
        signal_buffer_free(record->user_record);
    }

    free(record);
}
