/* Bare-metal shims for the QuickJS C engine (the `quickjs` feature).
 *
 * The espup newlib we link against expects the OS to supply a couple of hooks
 * that esp-idf normally provides but a bare esp-hal build does not. Defining
 * them here (compiled with the same toolchain) keeps the struct layouts correct.
 */
#include <time.h>
#include <reent.h>
#include <sys/stat.h>
#include <sys/time.h>

/* Time sources, defined in Rust (src/quickjs.rs): esp-hal monotonic timer and
 * beet's SNTP-disciplined wall clock. */
extern unsigned long long beet_esp_monotonic_ns(void);
extern long long beet_esp_wall_us(void);

/* newlib's stdio (printf/snprintf, used heavily by QuickJS) reaches its
 * per-thread state through __getreent. We run a single JS runtime, so hand back
 * the global reentrancy struct. */
struct _reent *__getreent(void) { return _impure_ptr; }

/* newlib's stdio bottoms out in these POSIX syscall stubs, which esp-idf
 * normally supplies. A bare esp-hal build has no filesystem or console wired to
 * them, so they fail or no-op: writes are discarded (QuickJS scripts return
 * values rather than print), reads/seeks fail, and there is no sbrk heap
 * (QuickJS allocates through Rust's global allocator via rquickjs `rust-alloc`). */
int _close(int fd) { (void)fd; return -1; }
int _fstat(int fd, struct stat *st) { (void)fd; if (st) st->st_mode = S_IFCHR; return 0; }
int _isatty(int fd) { (void)fd; return 1; }
int _lseek(int fd, int off, int whence) { (void)fd; (void)off; (void)whence; return -1; }
int _read(int fd, char *buf, int len) { (void)fd; (void)buf; (void)len; return 0; }
int _write(int fd, const char *buf, int len) { (void)fd; (void)buf; return len; }
void *_sbrk(int incr) { (void)incr; return (void *)-1; }
int _gettimeofday(struct timeval *tv, void *tz) {
    (void)tz;
    if (tv) {
        long long us = beet_esp_wall_us();
        tv->tv_sec = (time_t)(us / 1000000);
        tv->tv_usec = (long)(us % 1000000);
    }
    return 0;
}

/* QuickJS times its internal monotonic clock through clock_gettime and the wall
 * clock through gettimeofday, neither of which esp newlib provides. Both defer
 * to the Rust hooks above; C just marshals into the newlib structs. */
int clock_gettime(clockid_t clk_id, struct timespec *tp) {
    (void)clk_id;
    unsigned long long ns = beet_esp_monotonic_ns();
    tp->tv_sec = (time_t)(ns / 1000000000ULL);
    tp->tv_nsec = (long)(ns % 1000000000ULL);
    return 0;
}
