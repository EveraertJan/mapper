// A stand-in for the NDI runtime, for tests and demos without NDI installed.
//
// Implements the handful of NDIlib_* functions RUSTMAP loads, with the same
// struct layouts as NDI SDK 6.1. It advertises one source, "MOCK (Test
// Pattern)", that produces a moving gradient, and records what is sent.
//
// Build: cc -shared -fPIC -O2 -o libndi_mock.so mock_ndi.c
// Use:   RUSTMAP_NDI_LIB=/path/to/libndi_mock.so cargo run

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef struct { const char* p_ndi_name; const char* p_url_address; } NDIlib_source_t;
typedef struct { bool show_local_sources; const char* p_groups; const char* p_extra_ips; } NDIlib_find_create_t;
typedef struct {
    NDIlib_source_t source_to_connect_to;
    uint32_t color_format;
    int32_t bandwidth;
    bool allow_video_fields;
    const char* p_ndi_recv_name;
} NDIlib_recv_create_v3_t;
typedef struct { const char* p_ndi_name; const char* p_groups; bool clock_video; bool clock_audio; } NDIlib_send_create_t;
typedef struct {
    int xres, yres;
    uint32_t FourCC;
    int frame_rate_N, frame_rate_D;
    float picture_aspect_ratio;
    uint32_t frame_format_type;
    int64_t timecode;
    uint8_t* p_data;
    int line_stride_in_bytes;
    const char* p_metadata;
    int64_t timestamp;
} NDIlib_video_frame_v2_t;

#define FOURCC(a, b, c, d) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16) | ((uint32_t)(d) << 24))
#define MOCK_SOURCE "MOCK (Test Pattern)"
#define W 64
#define H 32
#define STRIDE (W * 4 + 16)

static NDIlib_source_t g_sources[1] = {{MOCK_SOURCE, "127.0.0.1:5961"}};

// Counters the tests read back.
static volatile int g_recv_created, g_recv_destroyed, g_frames_freed;
static volatile int g_send_created, g_send_destroyed, g_frames_sent;
static volatile int g_last_send[5];  // xres, yres, stride, fourcc, first pixel (packed RGBA)
static char g_send_name[256];

static void sleep_ms(long ms) {
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

bool NDIlib_initialize(void) { return true; }
void NDIlib_destroy(void) {}
const char* NDIlib_version(void) { return "MOCK NDI 6.1"; }

void* NDIlib_find_create_v2(const NDIlib_find_create_t* p) { (void)p; return malloc(1); }
void NDIlib_find_destroy(void* f) { free(f); }
bool NDIlib_find_wait_for_sources(void* f, uint32_t timeout_ms) {
    (void)f;
    sleep_ms(timeout_ms < 50 ? timeout_ms : 50);
    return true;
}
const NDIlib_source_t* NDIlib_find_get_current_sources(void* f, uint32_t* count) {
    (void)f;
    *count = 1;
    return g_sources;
}

typedef struct { int valid; unsigned counter; } recv_t;

void* NDIlib_recv_create_v3(const NDIlib_recv_create_v3_t* p) {
    recv_t* r = calloc(1, sizeof(recv_t));
    // Only the advertised source exists, and only in the format RUSTMAP asks for.
    r->valid = p && p->source_to_connect_to.p_ndi_name &&
               strcmp(p->source_to_connect_to.p_ndi_name, MOCK_SOURCE) == 0 &&
               p->color_format == 2 && p->bandwidth == 100;
    g_recv_created++;
    return r;
}
void NDIlib_recv_destroy(void* p) { free(p); g_recv_destroyed++; }

uint32_t NDIlib_recv_capture_v2(void* p, NDIlib_video_frame_v2_t* v, void* audio, void* meta, uint32_t timeout_ms) {
    (void)audio; (void)meta;
    recv_t* r = p;
    sleep_ms(timeout_ms < 16 ? timeout_ms : 16);
    if (!r->valid || !v) return 0;  // NDIlib_frame_type_none
    uint8_t* data = malloc(STRIDE * H);
    for (int y = 0; y < H; y++) {
        for (int x = 0; x < W; x++) {
            uint8_t* px = data + y * STRIDE + x * 4;
            px[0] = (uint8_t)(x * 4);                      // R: gradient
            px[1] = (uint8_t)(y * 8);                      // G: gradient
            px[2] = (uint8_t)(r->counter * 3);             // B: animates
            px[3] = (x < W / 2) ? 255 : 128;               // right half half-transparent
        }
        memset(data + y * STRIDE + W * 4, 0xEE, STRIDE - W * 4);  // padding garbage
    }
    r->counter++;
    memset(v, 0, sizeof(*v));
    v->xres = W; v->yres = H;
    v->FourCC = FOURCC('R', 'G', 'B', 'A');
    v->frame_rate_N = 60000; v->frame_rate_D = 1001;
    v->picture_aspect_ratio = (float)W / H;
    v->frame_format_type = 1;
    v->p_data = data;
    v->line_stride_in_bytes = STRIDE;
    return 1;  // NDIlib_frame_type_video
}
void NDIlib_recv_free_video_v2(void* p, const NDIlib_video_frame_v2_t* v) {
    (void)p;
    free(v->p_data);
    g_frames_freed++;
}

void* NDIlib_send_create(const NDIlib_send_create_t* p) {
    strncpy(g_send_name, p && p->p_ndi_name ? p->p_ndi_name : "", sizeof g_send_name - 1);
    g_send_created++;
    return malloc(1);
}
void NDIlib_send_destroy(void* p) { free(p); g_send_destroyed++; }
void NDIlib_send_send_video_v2(void* p, const NDIlib_video_frame_v2_t* v) {
    (void)p;
    g_last_send[0] = v->xres;
    g_last_send[1] = v->yres;
    g_last_send[2] = v->line_stride_in_bytes;
    g_last_send[3] = (int)v->FourCC;
    g_last_send[4] = v->p_data ? (int)(v->p_data[0] | v->p_data[1] << 8 | v->p_data[2] << 16 | (uint32_t)v->p_data[3] << 24) : 0;
    sleep_ms(2);  // pretend to compress
    g_frames_sent++;
}

// ---- test introspection (not part of NDI) ----
int mock_counter(int which) {
    switch (which) {
        case 0: return g_recv_created;
        case 1: return g_recv_destroyed;
        case 2: return g_frames_freed;
        case 3: return g_send_created;
        case 4: return g_send_destroyed;
        case 5: return g_frames_sent;
        default: return -1;
    }
}
int mock_last_send(int i) { return (i >= 0 && i < 5) ? g_last_send[i] : -1; }
const char* mock_send_name(void) { return g_send_name; }
