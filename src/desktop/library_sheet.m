#import <AppKit/AppKit.h>

typedef void (*Selection)(void *, NSInteger);

@interface TextampLibrarySheet : NSObject
@property (strong) NSPanel *panel;
@property (strong) NSStackView *stack;
@property (assign) void *context;
@property (assign) Selection selected;
@end

@implementation TextampLibrarySheet
- (void)choose:(NSControl *)sender {
    NSInteger index = [sender isKindOfClass:NSSegmentedControl.class]
        ? ((NSSegmentedControl *)sender).selectedSegment : sender.tag;
    self.selected(self.context, index);
}
@end

void *textamp_library_sheet_open(NSView *parent, void *context, Selection selected) {
    if (!parent.window) return NULL;
    TextampLibrarySheet *sheet = [TextampLibrarySheet new];
    sheet.context = context;
    sheet.selected = selected;
    sheet.panel = [[NSPanel alloc] initWithContentRect:NSMakeRect(0,0,440,240)
        styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    sheet.stack = [NSStackView new];
    sheet.stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    sheet.stack.alignment = NSLayoutAttributeLeading;
    sheet.stack.spacing = 10;
    sheet.stack.translatesAutoresizingMaskIntoConstraints = NO;
    NSView *content = sheet.panel.contentView;
    [content addSubview:sheet.stack];
    [NSLayoutConstraint activateConstraints:@[
        [sheet.stack.topAnchor constraintEqualToAnchor:content.topAnchor constant:18],
        [sheet.stack.leadingAnchor constraintEqualToAnchor:content.leadingAnchor constant:20],
        [sheet.stack.trailingAnchor constraintLessThanOrEqualToAnchor:content.trailingAnchor constant:-20]
    ]];
    [parent.window beginSheet:sheet.panel completionHandler:nil];
    return (__bridge_retained void *)sheet;
}

void textamp_library_sheet_update(void *handle, const char *json) {
    TextampLibrarySheet *sheet = (__bridge TextampLibrarySheet *)handle;
    NSDictionary *model = [NSJSONSerialization JSONObjectWithData:
        [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
    sheet.panel.title = model[@"title"];
    for (NSView *v in sheet.stack.arrangedSubviews.copy) [v removeFromSuperview];
    NSArray *tabs = model[@"tabs"];
    NSSegmentedControl *pages = [NSSegmentedControl segmentedControlWithLabels:tabs
        trackingMode:NSSegmentSwitchTrackingSelectOne target:sheet action:@selector(choose:)];
    pages.selectedSegment = [model[@"page"] integerValue];
    [sheet.stack addArrangedSubview:pages];
    for (NSString *line in model[@"details"])
        [sheet.stack addArrangedSubview:[NSTextField labelWithString:line]];
    NSInteger index = tabs.count;
    for (NSDictionary *control in model[@"controls"]) {
        NSButton *button;
        if (control[@"checked"]) {
            button = [NSButton checkboxWithTitle:control[@"label"] target:sheet action:@selector(choose:)];
            button.state = [control[@"checked"] boolValue] ? NSControlStateValueOn : NSControlStateValueOff;
        } else {
            button = [NSButton buttonWithTitle:control[@"label"] target:sheet action:@selector(choose:)];
        }
        button.tag = index++;
        [sheet.stack addArrangedSubview:button];
    }
    NSButton *done = [NSButton buttonWithTitle:@"Done" target:sheet action:@selector(choose:)];
    done.tag = -1;
    done.keyEquivalent = @"\033";
    [sheet.stack addArrangedSubview:done];
    [sheet.panel setContentSize:NSMakeSize(MAX(380,sheet.stack.fittingSize.width+40),sheet.stack.fittingSize.height+36)];
}

void textamp_library_sheet_close(void *handle) {
    TextampLibrarySheet *sheet = (__bridge_transfer TextampLibrarySheet *)handle;
    [sheet.panel.sheetParent endSheet:sheet.panel];
    [sheet.panel orderOut:nil];
}
