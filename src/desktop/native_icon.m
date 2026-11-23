#import <AppKit/AppKit.h>

// Called on the app thread after the active theme's image has been rendered.
unsigned int textamp_set_theme_icon(const unsigned char *bytes, size_t length) {
    @autoreleasepool {
        if (![NSThread isMainThread] || !bytes || !length) return 0;
        NSData *data = [NSData dataWithBytes:bytes length:length];
        NSImage *image = [[NSImage alloc] initWithData:data];
        if (!image) return 0;
        [NSApp setApplicationIconImage:image];
        unsigned int applied = 1;
        NSString *bundle = [[NSBundle mainBundle] bundlePath];
        if ([[bundle pathExtension] isEqualToString:@"app"] &&
            [[NSWorkspace sharedWorkspace] setIcon:image forFile:bundle options:0]) {
            applied |= 2;
        }
        return applied;
    }
}

unsigned int textamp_system_theme_colors(float *rgba) {
    @autoreleasepool {
        if (![NSThread isMainThread] || !NSApp || !rgba) return 0;
        [NSApp.effectiveAppearance performAsCurrentDrawingAppearance:^{
            NSColor *colors[] = {
                NSColor.windowBackgroundColor, NSColor.controlBackgroundColor,
                NSColor.textBackgroundColor, NSColor.labelColor,
                NSColor.secondaryLabelColor, NSColor.separatorColor,
                NSColor.controlAccentColor, NSColor.selectedContentBackgroundColor,
                NSColor.alternateSelectedControlTextColor, NSColor.unemphasizedSelectedContentBackgroundColor,
                NSColor.keyboardFocusIndicatorColor, NSColor.systemRedColor,
                NSColor.systemGreenColor, NSColor.systemOrangeColor
            };
            for (unsigned int i = 0; i < 14; i++) {
                NSColor *color = [colors[i] colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
                rgba[i * 4] = color.redComponent;
                rgba[i * 4 + 1] = color.greenComponent;
                rgba[i * 4 + 2] = color.blueComponent;
                rgba[i * 4 + 3] = color.alphaComponent;
            }
        }];
        return 14;
    }
}

static __weak NSWindow *textampLibraryWindow;
static NSHashTable<NSWindow *> *textampSuspendedWindows;
void textamp_register_library_view(void *view) {
    if (![NSThread isMainThread] || !view) return;
    textampLibraryWindow = [(__bridge NSView *)view window];
}
unsigned int textamp_platform_modal_active(void) {
    if (![NSThread isMainThread]) return 1;
    if (NSApp.modalWindow) return 1;
    for (NSWindow *window in NSApp.windows) {
        if (window.isVisible && ([window isKindOfClass:NSOpenPanel.class] ||
                                 [window isKindOfClass:NSSavePanel.class])) return 1;
        id responder = window.firstResponder;
        if (window.isKeyWindow && [responder conformsToProtocol:@protocol(NSTextInputClient)] &&
            [(id<NSTextInputClient>)responder hasMarkedText]) return 1;
    }
    return 0;
}
void textamp_suspend_auxiliary_windows(unsigned int suspend) {
    if (![NSThread isMainThread]) return;
    if (suspend) {
        if (!textampSuspendedWindows) textampSuspendedWindows = [NSHashTable weakObjectsHashTable];
        for (NSWindow *window in NSApp.windows) {
            if (window != textampLibraryWindow && window.isVisible && !window.isMiniaturized) {
                [textampSuspendedWindows addObject:window];
                [window orderOut:nil];
            }
        }
        [textampLibraryWindow makeKeyAndOrderFront:nil];
    } else {
        for (NSWindow *window in textampSuspendedWindows) [window orderFront:nil];
        [textampSuspendedWindows removeAllObjects];
    }
}
