#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#import <ApplicationServices/ApplicationServices.h>
#import <Security/Security.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <mach-o/dyld.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>

@interface FabushiSckShotBox : NSObject
@property (nonatomic, assign) BOOL timedOut;
@property (nonatomic, assign) CGImageRef capturedImage;
@property (nonatomic, strong) NSError *shotErr;
@end

@implementation FabushiSckShotBox
@end

// Status codes returned by fabushi_sck_capture_display & fabushi_sck_probe_access
enum {
    FABUSHI_SCK_OK = 0,
    FABUSHI_SCK_PERMISSION_DENIED = 1,
    FABUSHI_SCK_UNAVAILABLE = 2,
    FABUSHI_SCK_NOT_FOUND = 3,
    FABUSHI_SCK_ERROR = 4,
};

// Format enum matching Rust CaptureFormat
enum {
    FABUSHI_FMT_PNG = 0,
    FABUSHI_FMT_JPEG = 1,
    FABUSHI_FMT_BGRA8 = 2,
};

typedef struct {
    int32_t status;
    uint32_t width;
    uint32_t height;
    uint8_t *data;
    size_t data_len;
    char error_msg[512];
} FabushiSckCaptureResultC;

typedef struct {
    bool is_adhoc;
    bool is_app_bundle;
    char cdhash_hex[128];
    char bundle_id[256];
    char bundle_path[1024];
    char executable_path[1024];
    char team_id[128];
} FabushiMacSigningInfoC;

static void fabushi_copy_cstr(char *dst, size_t dst_size, NSString *src) {
    if (!dst || dst_size == 0) {
        return;
    }
    dst[0] = '\0';
    if (!src) {
        return;
    }
    const char *utf8 = [src UTF8String];
    if (!utf8) {
        return;
    }
    strlcpy(dst, utf8, dst_size);
}

static bool fabushi_is_tcc_denied_error(NSError *error) {
    if (!error) {
        return false;
    }
    // SCStreamErrorUserDeclined == -3801, SCStreamErrorFailedToStart == -3804,
    // SCStreamErrorMissingEntitlements == -3803
    if ([error.domain isEqualToString:@"com.apple.ScreenCaptureKit.SCStreamErrorDomain"] ||
        [error.domain containsString:@"SCStreamErrorDomain"]) {
        if (error.code == -3801 || error.code == -3803) {
            return true;
        }
    }
    NSString *desc = [[error localizedDescription] lowercaseString] ?: @"";
    if ([desc containsString:@"declined"] ||
        [desc containsString:@"tcc"] ||
        [desc containsString:@"permission"] ||
        [desc containsString:@"not authorized"] ||
        [desc containsString:@"拒绝"]) {
        return true;
    }
    return false;
}

static bool fabushi_encode_cgimage(
    CGImageRef image,
    int32_t format,
    uint8_t jpeg_quality,
    uint8_t **out_data,
    size_t *out_len,
    uint32_t *out_width,
    uint32_t *out_height,
    char *err_buf,
    size_t err_buf_len
) {
    if (!image) {
        strlcpy(err_buf, "CGImageRef was NULL", err_buf_len);
        return false;
    }
    size_t width = CGImageGetWidth(image);
    size_t height = CGImageGetHeight(image);
    if (width == 0 || height == 0) {
        strlcpy(err_buf, "Captured CGImage has zero dimensions", err_buf_len);
        return false;
    }
    *out_width = (uint32_t)width;
    *out_height = (uint32_t)height;

    if (format == FABUSHI_FMT_BGRA8) {
        size_t bytes_per_row = width * 4;
        size_t total_bytes = bytes_per_row * height;
        uint8_t *buf = (uint8_t *)malloc(total_bytes);
        if (!buf) {
            strlcpy(err_buf, "Out of memory allocating BGRA8 buffer", err_buf_len);
            return false;
        }
        CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
        CGBitmapInfo bitmapInfo = kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little;
        CGContextRef ctx = CGBitmapContextCreate(buf, width, height, 8, bytes_per_row, cs, bitmapInfo);
        CGColorSpaceRelease(cs);
        if (!ctx) {
            free(buf);
            strlcpy(err_buf, "Failed to create BGRA8 CGBitmapContext", err_buf_len);
            return false;
        }
        CGContextDrawImage(ctx, CGRectMake(0, 0, (CGFloat)width, (CGFloat)height), image);
        CGContextRelease(ctx);
        *out_data = buf;
        *out_len = total_bytes;
        return true;
    }

    CFMutableDataRef mutableData = CFDataCreateMutable(kCFAllocatorDefault, 0);
    if (!mutableData) {
        strlcpy(err_buf, "CFDataCreateMutable failed", err_buf_len);
        return false;
    }

    CFStringRef uti = (format == FABUSHI_FMT_PNG) ? CFSTR("public.png") : CFSTR("public.jpeg");
    CGImageDestinationRef dest = CGImageDestinationCreateWithData(mutableData, uti, 1, NULL);
    if (!dest) {
        CFRelease(mutableData);
        strlcpy(err_buf, "CGImageDestinationCreateWithData failed", err_buf_len);
        return false;
    }

    NSDictionary *props = nil;
    if (format == FABUSHI_FMT_JPEG) {
        double q = ((double)(jpeg_quality < 10 ? 10 : (jpeg_quality > 100 ? 100 : jpeg_quality))) / 100.0;
        props = @{ (__bridge NSString *)kCGImageDestinationLossyCompressionQuality: @(q) };
    }

    CGImageDestinationAddImage(dest, image, (__bridge CFDictionaryRef)props);
    bool finalized = CGImageDestinationFinalize(dest);
    CFRelease(dest);
    if (!finalized) {
        CFRelease(mutableData);
        strlcpy(err_buf, "CGImageDestinationFinalize failed", err_buf_len);
        return false;
    }

    CFIndex len = CFDataGetLength(mutableData);
    if (len <= 0) {
        CFRelease(mutableData);
        strlcpy(err_buf, "Encoded image buffer is empty", err_buf_len);
        return false;
    }

    uint8_t *copy = (uint8_t *)malloc((size_t)len);
    if (!copy) {
        CFRelease(mutableData);
        strlcpy(err_buf, "Out of memory copying encoded image bytes", err_buf_len);
        return false;
    }
    memcpy(copy, CFDataGetBytePtr(mutableData), (size_t)len);
    CFRelease(mutableData);

    *out_data = copy;
    *out_len = (size_t)len;
    return true;
}

void fabushi_sck_free_buffer(uint8_t *ptr) {
    if (ptr) {
        free(ptr);
    }
}

int32_t fabushi_sck_probe_access(char *err_buf, size_t err_buf_len) {
    if (err_buf && err_buf_len > 0) {
        err_buf[0] = '\0';
    }
    @autoreleasepool {
        // Never invoke SCShareableContent when TCC preflight is denied: on macOS 15+/27,
        // replayd escalates to a non-preflight TCCAccessRequest and pops up the OS consent
        // dialog even when our own permission modal is open.
        if (!CGPreflightScreenCaptureAccess()) {
            if (err_buf && err_buf_len > 0) {
                strlcpy(
                    err_buf,
                    "Screen Recording permission not granted (CGPreflightScreenCaptureAccess=false)",
                    err_buf_len
                );
            }
            return FABUSHI_SCK_PERMISSION_DENIED;
        }
        if (@available(macOS 12.3, *)) {
            Class scContentCls = NSClassFromString(@"SCShareableContent");
            if (!scContentCls) {
                if (err_buf && err_buf_len > 0) {
                    strlcpy(err_buf, "SCShareableContent unavailable at runtime", err_buf_len);
                }
                return FABUSHI_SCK_UNAVAILABLE;
            }

            dispatch_semaphore_t sem = dispatch_semaphore_create(0);
            __block int32_t result = FABUSHI_SCK_ERROR;
            __block NSString *errStr = nil;

            [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                                       onScreenWindowsOnly:YES
                                                         completionHandler:^(SCShareableContent * _Nullable content, NSError * _Nullable error) {
                if (error) {
                    errStr = [NSString stringWithFormat:@"%@ (%ld): %@",
                              error.domain ?: @"SCStreamErrorDomain",
                              (long)error.code,
                              error.localizedDescription ?: @"unknown error"];
                    result = fabushi_is_tcc_denied_error(error)
                        ? FABUSHI_SCK_PERMISSION_DENIED
                        : FABUSHI_SCK_ERROR;
                } else if (content && content.displays.count > 0) {
                    result = FABUSHI_SCK_OK;
                } else {
                    errStr = @"SCShareableContent returned no displays";
                    result = FABUSHI_SCK_NOT_FOUND;
                }
                dispatch_semaphore_signal(sem);
            }];

            long wait_rc = dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1800 * NSEC_PER_MSEC)));
            if (wait_rc != 0) {
                if (err_buf && err_buf_len > 0) {
                    strlcpy(err_buf, "SCShareableContent probe timed out", err_buf_len);
                }
                return FABUSHI_SCK_ERROR;
            }
            if (errStr && err_buf && err_buf_len > 0) {
                fabushi_copy_cstr(err_buf, err_buf_len, errStr);
            }
            return result;
        }
        if (err_buf && err_buf_len > 0) {
            strlcpy(err_buf, "ScreenCaptureKit requires macOS 12.3+", err_buf_len);
        }
        return FABUSHI_SCK_UNAVAILABLE;
    }
}

FabushiSckCaptureResultC fabushi_sck_capture_display(
    uint32_t target_display_id,
    uint32_t display_index,
    int32_t format,
    uint8_t jpeg_quality
) {
    FabushiSckCaptureResultC out;
    memset(&out, 0, sizeof(out));
    out.status = FABUSHI_SCK_UNAVAILABLE;

    @autoreleasepool {
        // Guard before calling SCShareableContent so missing Screen Recording permission
        // returns PermissionDenied cleanly instead of triggering macOS's intrusive OS popup.
        if (!CGPreflightScreenCaptureAccess()) {
            strlcpy(
                out.error_msg,
                "Screen Recording permission not granted (CGPreflightScreenCaptureAccess=false)",
                sizeof(out.error_msg)
            );
            out.status = FABUSHI_SCK_PERMISSION_DENIED;
            return out;
        }
        if (@available(macOS 14.0, *)) {
            Class scContentCls = NSClassFromString(@"SCShareableContent");
            Class scShotCls = NSClassFromString(@"SCScreenshotManager");
            if (!scContentCls || !scShotCls) {
                strlcpy(out.error_msg, "ScreenCaptureKit SCScreenshotManager unavailable", sizeof(out.error_msg));
                out.status = FABUSHI_SCK_UNAVAILABLE;
                return out;
            }

            dispatch_semaphore_t contentSem = dispatch_semaphore_create(0);
            __block SCShareableContent *capturedContent = nil;
            __block NSError *contentErr = nil;

            [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                                       onScreenWindowsOnly:YES
                                                         completionHandler:^(SCShareableContent * _Nullable content, NSError * _Nullable error) {
                capturedContent = content;
                contentErr = error;
                dispatch_semaphore_signal(contentSem);
            }];

            if (dispatch_semaphore_wait(contentSem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(3000 * NSEC_PER_MSEC))) != 0) {
                strlcpy(out.error_msg, "Timed out querying SCShareableContent", sizeof(out.error_msg));
                out.status = FABUSHI_SCK_ERROR;
                return out;
            }

            if (contentErr) {
                NSString *msg = [NSString stringWithFormat:@"ScreenCaptureKit shareable content error [%@ %ld]: %@",
                                 contentErr.domain ?: @"unknown",
                                 (long)contentErr.code,
                                 contentErr.localizedDescription ?: @"unknown"];
                fabushi_copy_cstr(out.error_msg, sizeof(out.error_msg), msg);
                out.status = fabushi_is_tcc_denied_error(contentErr)
                    ? FABUSHI_SCK_PERMISSION_DENIED
                    : FABUSHI_SCK_ERROR;
                return out;
            }

            NSArray<SCDisplay *> *displays = capturedContent.displays;
            if (!displays || displays.count == 0) {
                strlcpy(out.error_msg, "ScreenCaptureKit returned zero displays", sizeof(out.error_msg));
                out.status = FABUSHI_SCK_NOT_FOUND;
                return out;
            }

            SCDisplay *selected = nil;
            if (target_display_id != 0) {
                for (SCDisplay *d in displays) {
                    if (d.displayID == target_display_id) {
                        selected = d;
                        break;
                    }
                }
                if (!selected) {
                    snprintf(
                        out.error_msg,
                        sizeof(out.error_msg),
                        "Requested display ID %u not found among %lu active displays",
                        target_display_id,
                        (unsigned long)displays.count
                    );
                    out.status = FABUSHI_SCK_NOT_FOUND;
                    return out;
                }
            } else if ((NSUInteger)display_index < displays.count) {
                selected = displays[display_index];
            } else {
                snprintf(
                    out.error_msg,
                    sizeof(out.error_msg),
                    "Requested display index %u out of range (%lu displays)",
                    display_index,
                    (unsigned long)displays.count
                );
                out.status = FABUSHI_SCK_NOT_FOUND;
                return out;
            }

            SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:selected excludingWindows:@[]];
            SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];

            CGDisplayModeRef mode = CGDisplayCopyDisplayMode(selected.displayID);
            size_t pixelW = mode ? CGDisplayModeGetPixelWidth(mode) : (size_t)selected.width;
            size_t pixelH = mode ? CGDisplayModeGetPixelHeight(mode) : (size_t)selected.height;
            if (mode) {
                CGDisplayModeRelease(mode);
            }
            if (pixelW == 0) pixelW = (size_t)selected.width;
            if (pixelH == 0) pixelH = (size_t)selected.height;

            if (pixelW == 0 || pixelH == 0) {
                strlcpy(out.error_msg, "Selected display reported zero pixel dimensions", sizeof(out.error_msg));
                out.status = FABUSHI_SCK_ERROR;
                return out;
            }

            // Cap ultra-high retina width to keep remote frames crisp and fast while preserving aspect ratio
            if (pixelW > 2560 && pixelH > 0) {
                double scale = 2560.0 / (double)pixelW;
                pixelW = 2560;
                pixelH = (size_t)((double)pixelH * scale);
                if (pixelH < 1) pixelH = 1;
            }

            config.width = pixelW;
            config.height = pixelH;
            config.showsCursor = YES;
            config.scalesToFit = YES;

            dispatch_semaphore_t shotSem = dispatch_semaphore_create(0);
            FabushiSckShotBox *shotBox = [[FabushiSckShotBox alloc] init];

            [SCScreenshotManager captureImageWithFilter:filter
                                          configuration:config
                                      completionHandler:^(CGImageRef _Nullable image, NSError * _Nullable error) {
                @synchronized(shotBox) {
                    if (!shotBox.timedOut) {
                        if (image) {
                            shotBox.capturedImage = CGImageRetain(image);
                        }
                        shotBox.shotErr = error;
                    }
                }
                dispatch_semaphore_signal(shotSem);
            }];

            if (dispatch_semaphore_wait(shotSem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(4000 * NSEC_PER_MSEC))) != 0) {
                CGImageRef staleImage = NULL;
                @synchronized(shotBox) {
                    shotBox.timedOut = YES;
                    staleImage = shotBox.capturedImage;
                    shotBox.capturedImage = NULL;
                }
                if (staleImage) {
                    CGImageRelease(staleImage);
                }
                strlcpy(out.error_msg, "Timed out capturing screenshot via SCScreenshotManager", sizeof(out.error_msg));
                out.status = FABUSHI_SCK_ERROR;
                return out;
            }

            CGImageRef capturedImage = NULL;
            NSError *shotErr = nil;
            @synchronized(shotBox) {
                capturedImage = shotBox.capturedImage;
                shotBox.capturedImage = NULL;
                shotErr = shotBox.shotErr;
            }

            if (shotErr || !capturedImage) {
                if (capturedImage) {
                    CGImageRelease(capturedImage);
                }
                if (shotErr) {
                    NSString *msg = [NSString stringWithFormat:@"SCScreenshotManager failed [%@ %ld]: %@",
                                     shotErr.domain ?: @"unknown",
                                     (long)shotErr.code,
                                     shotErr.localizedDescription ?: @"unknown"];
                    fabushi_copy_cstr(out.error_msg, sizeof(out.error_msg), msg);
                    out.status = fabushi_is_tcc_denied_error(shotErr)
                        ? FABUSHI_SCK_PERMISSION_DENIED
                        : FABUSHI_SCK_ERROR;
                } else {
                    strlcpy(out.error_msg, "SCScreenshotManager returned NULL image", sizeof(out.error_msg));
                    out.status = FABUSHI_SCK_ERROR;
                }
                return out;
            }

            bool ok = fabushi_encode_cgimage(
                capturedImage,
                format,
                jpeg_quality,
                &out.data,
                &out.data_len,
                &out.width,
                &out.height,
                out.error_msg,
                sizeof(out.error_msg)
            );
            CGImageRelease(capturedImage);

            out.status = ok ? FABUSHI_SCK_OK : FABUSHI_SCK_ERROR;
            return out;
        }

        strlcpy(out.error_msg, "SCScreenshotManager requires macOS 14.0+", sizeof(out.error_msg));
        out.status = FABUSHI_SCK_UNAVAILABLE;
        return out;
    }
}

bool fabushi_cg_can_read_other_window_names(void) {
    @autoreleasepool {
        CFArrayRef windowList = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID
        );
        if (!windowList) {
            return false;
        }
        pid_t selfPid = getpid();
        bool foundNamedOtherWindow = false;
        NSArray *windows = CFBridgingRelease(windowList);
        for (NSDictionary *win in windows) {
            NSNumber *pidNum = win[(__bridge NSString *)kCGWindowOwnerPID];
            NSNumber *layerNum = win[(__bridge NSString *)kCGWindowLayer];
            if (!pidNum || pidNum.intValue == selfPid) {
                continue;
            }
            if (layerNum && layerNum.intValue != 0) {
                continue;
            }
            NSString *owner = win[(__bridge NSString *)kCGWindowOwnerName];
            if ([owner isEqualToString:@"Window Server"] ||
                [owner isEqualToString:@"WindowServer"] ||
                [owner isEqualToString:@"Dock"] ||
                [owner isEqualToString:@"程序坞"]) {
                continue;
            }
            NSString *name = win[(__bridge NSString *)kCGWindowName];
            if (name && name.length > 0) {
                foundNamedOtherWindow = true;
                break;
            }
        }
        return foundNamedOtherWindow;
    }
}

static void fabushi_fill_basic_bundle_info(FabushiMacSigningInfoC *info) {
    if (!info) {
        return;
    }
    // Resolve executable path
    uint32_t pathSize = (uint32_t)sizeof(info->executable_path);
    char rawExec[1024];
    memset(rawExec, 0, sizeof(rawExec));
    if (_NSGetExecutablePath(rawExec, &pathSize) == 0) {
        char resolved[PATH_MAX];
        if (realpath(rawExec, resolved)) {
            strlcpy(info->executable_path, resolved, sizeof(info->executable_path));
        } else {
            strlcpy(info->executable_path, rawExec, sizeof(info->executable_path));
        }
    }

    // Resolve .app bundle path & bundle ID without calling Security.framework (safe on main thread)
    NSBundle *mainBundle = [NSBundle mainBundle];
    NSString *bundlePath = mainBundle.bundleURL.path;
    NSString *bundleId = mainBundle.bundleIdentifier;
    if (bundlePath && [[bundlePath pathExtension] caseInsensitiveCompare:@"app"] == NSOrderedSame) {
        info->is_app_bundle = true;
        fabushi_copy_cstr(info->bundle_path, sizeof(info->bundle_path), bundlePath);
    } else {
        // Walk up executable_path in case we're inside foo.app/Contents/MacOS/...
        NSString *execStr = [NSString stringWithUTF8String:info->executable_path];
        NSString *cursor = execStr;
        while (cursor.length > 1) {
            if ([[cursor pathExtension] caseInsensitiveCompare:@"app"] == NSOrderedSame) {
                info->is_app_bundle = true;
                fabushi_copy_cstr(info->bundle_path, sizeof(info->bundle_path), cursor);
                NSBundle *b = [NSBundle bundleWithPath:cursor];
                if (b.bundleIdentifier.length > 0 && bundleId.length == 0) {
                    bundleId = b.bundleIdentifier;
                }
                break;
            }
            NSString *parent = [cursor stringByDeletingLastPathComponent];
            if ([parent isEqualToString:cursor]) {
                break;
            }
            cursor = parent;
        }
        if (!info->is_app_bundle) {
            strlcpy(info->bundle_path, info->executable_path, sizeof(info->bundle_path));
        }
    }
    if (bundleId.length > 0) {
        fabushi_copy_cstr(info->bundle_id, sizeof(info->bundle_id), bundleId);
    }
}

FabushiMacSigningInfoC fabushi_copy_basic_bundle_info(void) {
    FabushiMacSigningInfoC info;
    memset(&info, 0, sizeof(info));
    @autoreleasepool {
        fabushi_fill_basic_bundle_info(&info);
    }
    return info;
}

FabushiMacSigningInfoC fabushi_copy_signing_info(void) {
    FabushiMacSigningInfoC info;
    memset(&info, 0, sizeof(info));

    @autoreleasepool {
        fabushi_fill_basic_bundle_info(&info);

        // Inspect dynamic & static code signature via Security.framework (call off main thread)
        SecCodeRef selfCode = NULL;
        OSStatus status = SecCodeCopySelf(kSecCSDefaultFlags, &selfCode);
        if (status == errSecSuccess && selfCode) {
            CFDictionaryRef signingDict = NULL;
            status = SecCodeCopySigningInformation(
                selfCode,
                (SecCSFlags)(kSecCSDynamicInformation | kSecCSSigningInformation),
                &signingDict
            );
            if (status == errSecSuccess && signingDict) {
                NSDictionary *dict = CFBridgingRelease(signingDict);

                NSNumber *flagsNum = dict[(__bridge NSString *)kSecCodeInfoFlags];
                if (flagsNum) {
                    uint32_t flags = flagsNum.unsignedIntValue;
                    // kSecCodeSignatureAdhoc == 0x0002
                    info.is_adhoc = ((flags & 0x0002u) != 0);
                }

                NSString *teamId = dict[(__bridge NSString *)kSecCodeInfoTeamIdentifier];
                if (teamId.length > 0) {
                    fabushi_copy_cstr(info.team_id, sizeof(info.team_id), teamId);
                }

                if (info.bundle_id[0] == '\0') {
                    NSString *ident = dict[(__bridge NSString *)kSecCodeInfoIdentifier];
                    if (ident.length > 0) {
                        fabushi_copy_cstr(info.bundle_id, sizeof(info.bundle_id), ident);
                    }
                }

                NSData *cdhash = dict[(__bridge NSString *)kSecCodeInfoUnique];
                if ([cdhash isKindOfClass:[NSData class]] && cdhash.length > 0) {
                    const uint8_t *bytes = (const uint8_t *)cdhash.bytes;
                    size_t n = cdhash.length;
                    if (n * 2 >= sizeof(info.cdhash_hex)) {
                        n = (sizeof(info.cdhash_hex) - 1) / 2;
                    }
                    static const char hex[] = "0123456789abcdef";
                    for (size_t i = 0; i < n; i++) {
                        info.cdhash_hex[i * 2] = hex[(bytes[i] >> 4) & 0xF];
                        info.cdhash_hex[i * 2 + 1] = hex[bytes[i] & 0xF];
                    }
                    info.cdhash_hex[n * 2] = '\0';
                }
            }
            CFRelease(selfCode);
        }
    }

    return info;
}
