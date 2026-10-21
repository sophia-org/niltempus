/* Per-frame journal shared by the reference clients (rev 5: workload fields
 * for the idle/bursty/retained/fresh matrix, see WORKLOADS.md; rev 4:
 * lifecycle field for apples-to-apples pairs; rev 3 rows resolved with Muse 2026-10-21;
 * Muse amendments in rev 2).
 *
 * lifecycle=fresh: a new sealed memfd per frame (memfd_create, fallocate,
 * seal GROW|SHRINK, map, fill, unmap, seal WRITE|SEAL), passed to the
 * server and released when the server is done: the Sophia ISB lifecycle.
 * lifecycle=reuse: a fixed set of buffers reused under release events: the
 * usual Wayland/X practice and the D4 option B shape. Only arms with equal
 * lifecycle are joined.
 *
 * Line 1 is the header, which account.py must match across arms in every
 * field except `arm` and the `presented_clock` note:
 *   # header arm=<name> width=W height=H buffers=N fill=shade_mod128_a255 \
 *     frames=F warmup=U bulk=0|1 bulk_width=.. bulk_height=.. \
 *     lifecycle=fresh|reuse workload=idle|bursty|retained|fresh tiles=T \
 *     interval_ms=I damage=full|WxH+X+Y idle_sec=S clock=CLOCK_MONOTONIC_ns presented_clock=<note>
 *
 * Warmup frames (1..warmup) are submitted and paced exactly like counted
 * frames but write no rows. Then one TSV line per counted frame, indices
 * 1..frames contiguous in submission order; a counted frame whose
 * presentation never arrived keeps its row with presented_ns = -1 (never
 * renumbered). Fill generation: warmup frame w uses w, counted frame k uses
 * warmup+k, matching the Sophia peer (whose native join drops warmup rows):
 *   frame  fill_start_ns  submit_ns  presented_ns  pace_ns  presented_source
 *
 * All timestamps are CLOCK_MONOTONIC nanoseconds as read by the client.
 * pace_ns is the client-received timestamp of the event that released this
 * frame for submission (wl_callback.done on Wayland; the previous frame's
 * Present COMPLETE_NOTIFY receipt in X present mode; this frame's NotifyMSC
 * receipt in X putimage mode), or -1 where the arm has none.
 * presented_source names where presented_ns came from:
 *   wp_presentation   wp_presentation_feedback.presented, recorded as-is
 *                     because the compositor presents in CLOCK_MONOTONIC
 *   xpresent          Present COMPLETE_NOTIFY ust (server clock, usec) * 1000;
 *                     the X server reports CLOCK_MONOTONIC usec on DRM
 *                     backends; account.py checks submit <= ust <= receipt
 *   xpresent_recv     client receipt time of that COMPLETE_NOTIFY (fallback
 *                     when the ust check fails)
 *   shm_completion    client receipt time of ShmCompletion (server copy done,
 *                     not a scanout time)
 * No conversion between clocks happens anywhere; if a platform ever needs
 * one it is done once at capture from a monotonic pairing and recorded in
 * the presented_clock header note.
 *
 * Workload fields: tiles=0 unless retained; interval_ms=0 unless bursty;
 * damage=full unless bursty (96x24+1824+0); idle_sec=0 unless idle.
 *
 * Trailing metadata lines:
 *   # rusage utime_ns=.. stime_ns=.. minflt=.. majflt=..
 *   # ctxsw voluntary=.. involuntary=..        (client getrusage)
 *   # pid <client pid>
 *   # presented_clock_check ok=N violations=M   (X only)
 */
#ifndef EXTERNAL_BAR_JOURNAL_H
#define EXTERNAL_BAR_JOURNAL_H
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

static inline uint64_t mono_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static inline void journal_header(FILE *j, const char *arm, unsigned width,
                                  unsigned height, unsigned buffers,
                                  unsigned frames, unsigned warmup, int bulk,
                                  unsigned bulk_width, unsigned bulk_height,
                                  const char *lifecycle, const char *workload,
                                  unsigned tiles, unsigned interval_ms,
                                  const char *damage, unsigned idle_sec,
                                  const char *presented_clock) {
    fprintf(j,
            "# header arm=%s width=%u height=%u buffers=%u fill=%s "
            "frames=%u warmup=%u bulk=%d bulk_width=%u bulk_height=%u "
            "lifecycle=%s workload=%s tiles=%u interval_ms=%u damage=%s "
            "idle_sec=%u clock=CLOCK_MONOTONIC_ns presented_clock=%s\n",
            arm, width, height, buffers,
            strcmp(workload, "bursty") == 0 ? "bursty_static_background_v1" : "shade_mod128_a255",
            frames, warmup, bulk, bulk_width,
            bulk_height, lifecycle, workload, tiles, interval_ms, damage,
            idle_sec, presented_clock);
}

static inline void journal_frame(FILE *j, uint64_t frame, uint64_t fill_start,
                                 uint64_t submit, int64_t presented,
                                 int64_t pace, const char *source) {
    fprintf(j, "%llu\t%llu\t%llu\t%lld\t%lld\t%s\n", (unsigned long long)frame,
            (unsigned long long)fill_start, (unsigned long long)submit,
            (long long)presented, (long long)pace, source);
}

/* Client footer: CPU, faults and context switches for the whole run. */
#include <sys/resource.h>
static inline void journal_footer_rusage(FILE *j) {
    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    fprintf(j, "# rusage utime_ns=%llu stime_ns=%llu minflt=%ld majflt=%ld\n",
            (unsigned long long)ru.ru_utime.tv_sec * 1000000000ull +
                (unsigned long long)ru.ru_utime.tv_usec * 1000ull,
            (unsigned long long)ru.ru_stime.tv_sec * 1000000000ull +
                (unsigned long long)ru.ru_stime.tv_usec * 1000ull,
            ru.ru_minflt, ru.ru_majflt);
    fprintf(j, "# ctxsw voluntary=%ld involuntary=%ld\n", ru.ru_nvcsw, ru.ru_nivcsw);
}

#endif
