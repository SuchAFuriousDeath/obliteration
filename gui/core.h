#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

#ifdef __linux__
#include <linux/kvm.h>
#endif

struct Param;
struct Pkg;

/**
 * Display resolution to report to the kernel.
 */
enum DisplayResolution {
    /**
     * 1280 × 720.
     */
    DisplayResolution_Hd,
    /**
     * 1920 × 1080.
     */
    DisplayResolution_FullHd,
    /**
     * 3840 × 2160.
     */
    DisplayResolution_UltraHd,
};

/**
 * Log category.
 *
 * The reason we need this because cbindgen is not good at exporting dependency types so we can't
 * use [`ConsoleType`] directly. See https://github.com/mozilla/cbindgen/issues/667 for an example
 * of the problem.
 */
enum VmmLog {
    VmmLog_Info,
    VmmLog_Warn,
    VmmLog_Error,
};

/**
 * Encapsulate a debugger connection.
 */
struct DebugClient;

/**
 * TCP listener to accept a debugger connection.
 */
struct DebugServer;

/**
 * Reason for [`VmmEvent::Breakpoint`].
 */
struct KernelStop;

/**
 * Contains settings to launch the kernel.
 */
struct Profile;

/**
 * Error object managed by Rust side.
 */
struct RustError;

/**
 * Manage a virtual machine that run the kernel.
 */
struct Vmm;

/**
 * Contains objects required to render the screen.
 */
struct VmmScreen {
#if !defined(__APPLE__)
    size_t vk_instance
#endif
    ;
#if !defined(__APPLE__)
    size_t vk_device
#endif
    ;
#if !defined(__APPLE__)
    size_t vk_surface
#endif
    ;
#if defined(__APPLE__)
    size_t view
#endif
    ;
};

/**
 * Contains VMM event information.
 */
enum VmmEvent_Tag {
    VmmEvent_Error,
    VmmEvent_Exiting,
    VmmEvent_Log,
    VmmEvent_Breakpoint,
};

struct VmmEvent_Error_Body {
    const struct RustError *reason;
};

struct VmmEvent_Exiting_Body {
    bool success;
};

struct VmmEvent_Log_Body {
    enum VmmLog ty;
    const char *data;
    size_t len;
};

struct VmmEvent_Breakpoint_Body {
    struct KernelStop *stop;
};

struct VmmEvent {
    enum VmmEvent_Tag tag;
    union {
        struct VmmEvent_Error_Body error;
        struct VmmEvent_Exiting_Body exiting;
        struct VmmEvent_Log_Body log;
        struct VmmEvent_Breakpoint_Body breakpoint;
    };
};

/**
 * Result of [`vmm_dispatch_debug()`].
 */
enum DebugResult_Tag {
    DebugResult_Ok,
    DebugResult_Disconnected,
    DebugResult_Error,
};

struct DebugResult_Error_Body {
    struct RustError *reason;
};

struct DebugResult {
    enum DebugResult_Tag tag;
    union {
        struct DebugResult_Error_Body error;
    };
};

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

void set_panic_hook(void *cx,
                    void (*hook)(const char*, size_t, uint32_t, const char*, size_t, void*));

struct DebugServer *debug_server_start(const char *addr, struct RustError **err);

void debug_server_free(struct DebugServer *s);

const char *debug_server_addr(struct DebugServer *s);

ptrdiff_t debug_server_socket(struct DebugServer *s);

struct DebugClient *debug_server_accept(struct DebugServer *s, struct RustError **err);

void debug_client_free(struct DebugClient *d);

void error_free(struct RustError *e);

const char *error_message(const struct RustError *e);

Param *param_open(const char *file, struct RustError **error);

void param_close(Param *p);

char *param_app_ver_get(const Param *p);

char *param_category_get(const Param *p);

char *param_content_id_get(const Param *p);

char *param_short_content_id_get(const Param *p);

char *param_title_get(const Param *p);

char *param_title_id_get(const Param *p);

char *param_version_get(const Param *p);

Pkg *pkg_open(const char *file, struct RustError **error);

void pkg_close(Pkg *pkg);

Param *pkg_get_param(const Pkg *pkg, struct RustError **error);

struct RustError *pkg_extract(const Pkg *pkg, const char *dir, void (*status)(const char*,
                                                                              size_t,
                                                                              uint64_t,
                                                                              uint64_t,
                                                                              void*), void *ud);

struct Profile *profile_new(const char *name);

struct Profile *profile_load(const char *path, struct RustError **err);

void profile_free(struct Profile *p);

char *profile_id(const struct Profile *p);

const char *profile_name(const struct Profile *p);

enum DisplayResolution profile_display_resolution(const struct Profile *p);

void profile_set_display_resolution(struct Profile *p, enum DisplayResolution v);

struct RustError *profile_save(const struct Profile *p, const char *path);

struct RustError *update_firmware(const char *root,
                                  const char *fw,
                                  void *cx,
                                  void (*status)(const char*, uint64_t, uint64_t, void*));

struct Vmm *vmm_start(const char *kernel,
                      const struct VmmScreen *screen,
                      const struct Profile *profile,
                      struct DebugClient *debugger,
                      void (*event)(const struct VmmEvent*, void*),
                      void *cx,
                      struct RustError **err);

void vmm_free(struct Vmm *vmm);

struct RustError *vmm_draw(struct Vmm *vmm);

struct DebugResult vmm_dispatch_debug(struct Vmm *vmm, struct KernelStop *stop);

ptrdiff_t vmm_debug_socket(struct Vmm *vmm);

void vmm_shutdown(struct Vmm *vmm);

bool vmm_shutting_down(struct Vmm *vmm);

#ifdef __cplusplus
} // extern "C"
#endif // __cplusplus
