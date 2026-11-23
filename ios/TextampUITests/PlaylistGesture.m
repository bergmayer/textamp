#import <XCTest/XCTest.h>
#import <UIKit/UIKit.h>

// XCTest exposes twoFingerTap but has no public equivalent for a two-finger pan.
// Event synthesis is confined to the test runner.
@protocol PlaylistTouchSynthesizing
- (instancetype)initForTouchAtPoint:(CGPoint)point offset:(double)offset;
- (instancetype)initWithName:(NSString *)name interfaceOrientation:(NSInteger)orientation;
- (void)addPointerEventPath:(id)path;
- (void)moveToPoint:(CGPoint)point atOffset:(double)offset;
- (void)liftUpAtOffset:(double)offset;
- (id)eventSynthesizer;
- (void)synthesizeEvent:(id)event completion:(void (^)(BOOL, NSError *))completion;
@end

@implementation XCTestCase (PlaylistGesture)
- (void)selectPlaylistRowsFrom:(NSValue *)startValue to:(NSValue *)endValue {
    CGPoint start = startValue.CGPointValue;
    CGPoint end = endValue.CGPointValue;
    id event = [(id<PlaylistTouchSynthesizing>)[NSClassFromString(@"XCSynthesizedEventRecord") alloc]
        initWithName:@"Select playlist rows" interfaceOrientation:UIInterfaceOrientationPortrait];
    for (int finger = 0; finger < 2; finger++) {
        CGFloat x = start.x + (finger == 0 ? -22 : 22);
        id path = [(id<PlaylistTouchSynthesizing>)[NSClassFromString(@"XCPointerEventPath") alloc]
            initForTouchAtPoint:CGPointMake(x, start.y) offset:0];
        for (int step = 1; step <= 12; step++) {
            [path moveToPoint:CGPointMake(x, start.y + (end.y - start.y) * step / 12)
                     atOffset:0.06 * step];
        }
        [path liftUpAtOffset:0.8];
        [event addPointerEventPath:path];
    }
    XCTestExpectation *sent = [self expectationWithDescription:@"Two-finger pan delivered"];
    id device = XCUIDevice.sharedDevice;
    [[device eventSynthesizer] synthesizeEvent:event completion:^(BOOL success, NSError *error) {
        XCTAssertTrue(success, @"%@", error);
        [sent fulfill];
    }];
    [self waitForExpectations:@[sent] timeout:5];
}
@end
