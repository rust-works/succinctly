/*
 * A thin, pointer-only interface over jq's own reader, so the Rust side never passes jq's `jv`
 * struct by value across the FFI boundary.
 *
 * build.rs compiles this file once per vendored release, with that release's rename header and
 * `-DSJQ_PREFIX=sjq171_` or `sjq182_`, so each export below is `sjq171_<name>` or
 * `sjq182_<name>` (src/ffi.rs).
 *
 * Every `*_next` returns 0 at end of input (an invalid without a message), 1 for a value and 2
 * for an error, and hands back the value's `jv_dump_string(v, 0)` text (what `jq -c` prints) or
 * the error message as a malloc'd buffer the caller frees with `free_buf`. A function that
 * allocates returns -1 (or NULL) when malloc fails; jq's own allocations abort on failure, as in
 * jq.
 */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "jq.h"
#include "jv.h"
#include "jv_unicode.h"

#define SJQ_CAT2(a, b) a##b
#define SJQ_CAT(a, b) SJQ_CAT2(a, b)
#define SJQ(name) SJQ_CAT(SJQ_PREFIX, name)

/*
 * util.c's position accessors take a `jq_state *` and ask the interpreter for its input callback.
 * With no interpreter linked, the "state" handed to them is the input state itself, and this
 * stand-in returns it as the callback data. (The rename header renames it with the rest.)
 */
void jq_get_input_cb(jq_state *jq, jq_input_cb *cb, void **data) {
  *cb = jq_util_input_next_input_cb;
  *data = (void *)jq;
}

static char *dup_bytes(const char *s, size_t n) {
  char *p = malloc(n + 1);
  if (p == NULL)
    return NULL;
  memcpy(p, s, n);
  p[n] = 0;
  return p;
}

/* Hand `v` (consumed) back as text: 1 value, 2 error message, 0 end of input, -1 out of memory. */
static int take(jv v, char **out, size_t *len) {
  *out = NULL;
  *len = 0;
  if (jv_is_valid(v)) {
    jv s = jv_dump_string(v, 0);
    *len = (size_t)jv_string_length_bytes(jv_copy(s));
    *out = dup_bytes(jv_string_value(s), *len);
    jv_free(s);
    return *out ? 1 : -1;
  }
  if (!jv_invalid_has_msg(jv_copy(v))) {
    jv_free(v);
    return 0;
  }
  jv msg = jv_invalid_get_msg(v);
  if (jv_get_kind(msg) != JV_KIND_STRING)
    msg = jv_dump_string(msg, 0);
  *len = (size_t)jv_string_length_bytes(jv_copy(msg));
  *out = dup_bytes(jv_string_value(msg), *len);
  jv_free(msg);
  return *out ? 2 : -1;
}

void SJQ(free_buf)(char *p) { free(p); }

/* ---- the real jq_util_input_* over named files ---- */

struct sjq_errs {
  char *buf;
  size_t len;
};

/* The text of util.c's default `fprinter`, captured instead of printed. */
static void err_cb(void *data, const char *fname) {
  struct sjq_errs *e = data;
  char line[4096];
  int n = snprintf(line, sizeof line, "jq: error: Could not open file %s: %s\n", fname,
                   strerror(errno));
  if (n < 0)
    return;
  if ((size_t)n >= sizeof line)
    n = sizeof line - 1;
  char *grown = realloc(e->buf, e->len + n + 1);
  if (grown == NULL)
    return;
  e->buf = grown;
  memcpy(e->buf + e->len, line, n);
  e->len += n;
  e->buf[e->len] = 0;
}

struct sjq_input {
  jq_util_input_state *st;
  struct sjq_errs errs;
};

/* raw: `-R` (no parser). flags: JV_PARSE_* bits. NULL when malloc fails. */
struct sjq_input *SJQ(input_new)(int flags, int raw, int slurp) {
  struct sjq_input *in = calloc(1, sizeof *in);
  if (in == NULL)
    return NULL;
  in->st = jq_util_input_init(err_cb, &in->errs);
  jq_util_input_set_parser(in->st, raw ? NULL : jv_parser_new(flags), slurp);
  return in;
}

void SJQ(input_add)(struct sjq_input *in, const char *fname) {
  jq_util_input_add_input(in->st, fname);
}

int SJQ(input_next)(struct sjq_input *in, char **out, size_t *len) {
  return take(jq_util_input_next_input(in->st), out, len);
}

int SJQ(input_errors)(struct sjq_input *in) { return jq_util_input_errors(in->st); }

/* Open-failure messages printed since the last call (malloc'd; NULL when none). */
char *SJQ(input_take_errs)(struct sjq_input *in, size_t *len) {
  char *p = in->errs.buf;
  *len = in->errs.len;
  in->errs.buf = NULL;
  in->errs.len = 0;
  return p;
}

/* `input_filename`: 0 when null (no file yet), 1 with the name, -1 out of memory. */
int SJQ(input_filename)(struct sjq_input *in, char **out, size_t *len) {
  jv f = jq_util_input_get_current_filename((jq_state *)in->st);
  *out = NULL;
  *len = 0;
  if (jv_get_kind(f) != JV_KIND_STRING) {
    jv_free(f);
    return 0;
  }
  *len = (size_t)jv_string_length_bytes(jv_copy(f));
  *out = dup_bytes(jv_string_value(f), *len);
  jv_free(f);
  return *out ? 1 : -1;
}

/* `input_line_number`. */
double SJQ(input_line)(struct sjq_input *in) {
  jv l = jq_util_input_get_current_line((jq_state *)in->st);
  double d = jv_get_kind(l) == JV_KIND_NUMBER ? jv_number_value(l) : -1;
  jv_free(l);
  return d;
}

void SJQ(input_free)(struct sjq_input *in) {
  jq_util_input_free(&in->st);
  free(in->errs.buf);
  free(in);
}

/* ---- the bare parser, for a reader that delivers its own chunks ---- */

jv_parser *SJQ(parser_new)(int flags) { return jv_parser_new(flags); }
void SJQ(parser_set_buf)(jv_parser *p, const char *buf, int len, int is_partial) {
  jv_parser_set_buf(p, buf, len, is_partial);
}
int SJQ(parser_remaining)(jv_parser *p) { return jv_parser_remaining(p); }
int SJQ(parser_next)(jv_parser *p, char **out, size_t *len) {
  return take(jv_parser_next(p), out, len);
}
void SJQ(parser_free)(jv_parser *p) { jv_parser_free(p); }

/* `jv_dump_string(jv_string_sized(s, len), 0)`: 1, or -1 out of memory. */
int SJQ(json_string)(const char *s, size_t len, char **out, size_t *out_len) {
  return take(jv_string_sized(s, (int)len), out, out_len) == 1 ? 1 : -1;
}

/* `jq_util_input_read_more`'s test (1.8.2): the bytes the last character of a full chunk is
 * missing, or 0. */
int SJQ(utf8_missing)(const char *last, const char *min) {
  int len = 0;
  if (jvp_utf8_backtrack(last, min, &len) && len > 0)
    return len;
  return 0;
}

/* The vendored 1.8.2 jv_parse.c's sync-point patch; 1.7.1 is built without it (-1). */
#ifndef SJQ_NO_SYNC
struct jv_parser_sync {
  int line, column;
  unsigned bom_strip_position;
  int waiting_for_rs;
  int last_ch_was_ws;
};
int jv_parser_sync_point(jv_parser *p, struct jv_parser_sync *s);
void jv_parser_seed(jv_parser *p, const struct jv_parser_sync *s);
int SJQ(parser_sync_point)(jv_parser *p, int *out5) {
  struct jv_parser_sync s;
  int r = jv_parser_sync_point(p, &s);
  out5[0] = s.line;
  out5[1] = s.column;
  out5[2] = (int)s.bom_strip_position;
  out5[3] = s.waiting_for_rs;
  out5[4] = s.last_ch_was_ws;
  return r;
}
void SJQ(parser_seed)(jv_parser *p, const int *in5) {
  struct jv_parser_sync s = {in5[0], in5[1], (unsigned)in5[2], in5[3], in5[4]};
  jv_parser_seed(p, &s);
}
#else
int SJQ(parser_sync_point)(jv_parser *p, int *out5) {
  (void)p;
  (void)out5;
  return -1;
}
void SJQ(parser_seed)(jv_parser *p, const int *in5) {
  (void)p;
  (void)in5;
}
#endif
