/* Complete C adapter calls; fixture data remains owned until every series closes. */
#define main embedding_original_main
#include "c_embedding.c"
#undef main
#include <time.h>

typedef struct {
    fixture base;
    uint64_t count, width;
    netdata_promql_label_t (*labels)[17];
    char (*ids)[24];
} capacity_source;

typedef struct {
    capacity_source *source;
    uint64_t rows;
    unsigned metadata, begun, committed, aborted;
} capacity_output;

static uint32_t capacity_open(void *context, void *query,
        const netdata_promql_selection_request_t *request,
        const netdata_promql_callback_control_t *control, void **out,
        netdata_promql_bytes_t *error) {
    (void)query; (void)request; (void)error;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    capacity_source *source = context;
    *out = calloc(1, sizeof(uint64_t));
    assert(*out);
    source->base.opened[1]++;
    return NETDATA_PROMQL_OK;
}

static netdata_promql_resources_t capacity_resources(void *context, void *handle,
        const netdata_promql_accounting_context_t *account) {
    netdata_promql_resources_t value = no_resources(context, handle, account);
    value.held_bytes = sizeof(uint64_t);
    return value;
}

static uint32_t capacity_next(void *context, void *handle,
        const netdata_promql_callback_control_t *control, netdata_promql_series_open_t *out,
        netdata_promql_provider_diagnostics_t *diagnostics, netdata_promql_bytes_t *error) {
    (void)diagnostics; (void)error;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    capacity_source *source = context;
    uint64_t *index = handle;
    if (*index == source->count) return NETDATA_PROMQL_END;
    out->handle = NULL;
    out->labels.data = source->labels[*index];
    out->labels.len = source->width;
    (*index)++;
    source->base.opened[2]++;
    return NETDATA_PROMQL_OK;
}

static uint32_t capacity_begin(void *context, const netdata_promql_output_header_t *header,
        const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    (void)error;
    capacity_output *output = context;
    assert(header->value_type == NETDATA_PROMQL_TYPE_VECTOR);
    assert(header->start_ms == 1000 && header->end_ms == 1000 && header->step_ms == 0);
    output->begun++;
    return checked_control(control);
}

static uint32_t capacity_chunk(void *context, const netdata_promql_chunk_t *chunk,
        const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    (void)error;
    capacity_output *output = context;
    capacity_source *source = output->source;
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES);
    assert(output->rows < source->count);
    assert(chunk->payload.series.series_id == output->rows);
    assert(chunk->payload.series.labels.len == source->width);
    for (uint64_t label = 0; label < source->width; label++)
        label_copy_equal(chunk->payload.series.labels.data[label], source->labels[output->rows][label]);
    if (output->metadata) assert(chunk->payload.series.points.len == 0);
    else {
        assert(chunk->payload.series.points.len == 1);
        const netdata_promql_sample_t *point = chunk->payload.series.points.data;
        assert(point->time_ms == 1000 && point->value.kind == NETDATA_PROMQL_VALUE_FLOAT);
        assert(point->value.payload.float_bits == netdata_promql_double_bits(1.0));
    }
    output->rows++;
    return checked_control(control);
}

static uint32_t capacity_commit(void *context, const netdata_promql_query_observations_t *observations,
        const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    (void)error;
    capacity_output *output = context;
    assert(output->rows == output->source->count && output->begun == 1);
    assert(observations->output_samples == (output->metadata ? 0 : output->rows));
    if (output->metadata) {
        assert(!observations->pages && !observations->samples_fetched);
        assert(!observations->statistics.samples.total_queryable_samples);
        assert(!observations->statistics.samples.samples_read && !observations->statistics.samples.peak_samples);
    }
    released(&output->source->base);
    output->committed++;
    return checked_control(control);
}

static void capacity_abort(void *context, const netdata_promql_accounting_context_t *account) {
    assert(account);
    ((capacity_output *)context)->aborted++;
}

static uint64_t capacity_clock(void) {
    struct timespec now;
    assert(clock_gettime(CLOCK_MONOTONIC, &now) == 0);
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

int main(int argc, char **argv) {
    assert(argc == 4 || argc == 2);
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    if (argc == 2) {
        assert(strcmp(argv[1], "--metadata-controls") == 0);
        metadata_discovery(engine);
        balanced(engine, &idle);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
        return 0;
    }
    capacity_source source;
    memset(&source, 0, sizeof(source));
    init_fixture(&source.base, 1, 1);
    source.width = strtoull(argv[1], NULL, 10);
    source.count = strtoull(argv[2], NULL, 10);
    unsigned metadata = strcmp(argv[3], "metadata") == 0;
    assert(metadata || strcmp(argv[3], "query") == 0);
    assert(source.width <= 17 && source.count && source.count <= 2048);
    assert(source.width || source.count == 1);
    source.labels = calloc((size_t)source.count, sizeof(*source.labels));
    source.ids = calloc((size_t)source.count, sizeof(*source.ids));
    assert(source.labels && source.ids);
    static const char *names[] = {"id", "k01", "k02", "k03", "k04", "k05", "k06", "k07", "k08",
                                  "k09", "k10", "k11", "k12", "k13", "k14", "k15", "k16"};
    static const uint8_t extra[] = {'x', 0, 255, 'y'};
    for (uint64_t row = 0; row < source.count; row++) {
        assert(snprintf(source.ids[row], sizeof(*source.ids), "%08" PRIu64, row) == 8);
        for (uint64_t label = 0; label < source.width; label++) {
            source.labels[row][label].name = text(names[label]);
            source.labels[row][label].value = label ? bytes(extra, sizeof(extra)) : text(source.ids[row]);
        }
    }
    netdata_promql_provider_t input = provider(&source.base);
    input.selection_open = capacity_open;
    input.selection_next = capacity_next;
    input.selection_resources = capacity_resources;
    netdata_promql_query_t *query = metadata ? NULL : compile(engine, source.width ? "{id=~\".+\"}" : "vector(1)");
    netdata_promql_engine_observations_t compiled = engine_observations(engine);
    for (unsigned repeat = 0; repeat < 4; repeat++) {
        capacity_output output = {.source = &source, .metadata = metadata};
        netdata_promql_output_t sink = {0};
        sink.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
        sink.header.struct_size = sizeof(sink);
        sink.context = &output;
        sink.begin = capacity_begin; sink.chunk = capacity_chunk;
        sink.commit = capacity_commit; sink.abort = capacity_abort;
        netdata_promql_report_t *report = NULL;
        netdata_promql_bounds_t bounds = {1000, 1000};
        netdata_promql_request_t request = instant(1000);
        uint64_t started = capacity_clock();
        uint32_t status = metadata ? netdata_promql_discover(engine, &input, bounds, NULL, 0, NULL, &sink, &report) :
                                    netdata_promql_execute(query, &input, &request, NULL, &sink, &report);
        uint64_t elapsed = capacity_clock() - started;
        assert(status == NETDATA_PROMQL_OK && output.committed == 1 && !output.aborted);
        netdata_promql_query_observations_t observed;
        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
        statistics_valid(&observed.statistics);
        printf("{\"mode\":\"%s\",\"width\":%" PRIu64 ",\"rows\":%" PRIu64 ",\"repeat\":%u,\"ns\":%" PRIu64
               ",\"calls\":%" PRIu64 ",\"pages\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"work\":%" PRIu64
               ",\"output_samples\":%" PRIu64 ",\"statistics\":[%" PRIu64 ",%" PRIu64 ",%" PRIu64 "],\"memory\":[",
               argv[3], source.width, source.count, repeat, elapsed, observed.provider_calls, observed.pages,
               observed.samples_fetched, observed.work, observed.output_samples,
               observed.statistics.samples.total_queryable_samples, observed.statistics.samples.samples_read,
               observed.statistics.samples.peak_samples);
        for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
            netdata_promql_memory_observation_t value = observed.memory[kind];
            assert(!value.overflowed && value.acquired - value.released == value.current);
            printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", kind ? "," : "",
                   value.current, value.peak, value.acquired, value.released);
        }
        puts("]}");
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        released(&source.base);
        balanced(engine, &compiled);
    }
    if (query) assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    free(source.labels);
    free(source.ids);
    return 0;
}
