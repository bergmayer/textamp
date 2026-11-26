// ImageCaptureCore owns the USB session. The helper process supplies its main run loop.
#import <Foundation/Foundation.h>
#import <ImageCaptureCore/ImageCaptureCore.h>

static void workerError(NSString *reason, BOOL reconnect) {
    NSData *error = [NSJSONSerialization dataWithJSONObject:@{@"error": reason, @"reconnect": @(reconnect)} options:0 error:nil];
    fwrite(error.bytes, 1, error.length, stdout);
    fputc('\n', stdout);
    fflush(stdout);
    exit(2);
}
static void fail(NSString *reason) { workerError(reason, NO); }
static void interrupted(NSString *reason) { workerError(reason, YES); }
static uint32_t number(NSData *data, NSUInteger offset, NSUInteger size) {
    if (offset > data.length || size > data.length - offset || size > 4) fail(@"Invalid MTP response");
    uint32_t value = 0;
    memcpy(&value, (const char *)data.bytes + offset, size);
    return value;
}
static NSData *requestStatus(ICCameraDevice *device, uint16_t op, NSArray<NSNumber *> *args,
                       NSData *output, NSData **response, uint16_t *result) {
    uint32_t header[] = { (uint32_t)(12 + 4 * args.count), ((uint32_t)op << 16) | 1, 0 };
    NSMutableData *command = [NSMutableData dataWithBytes:header length:sizeof(header)];
    for (NSNumber *arg in args) {
        uint32_t value = arg.unsignedIntValue;
        [command appendBytes:&value length:4];
    }
    __block NSData *body, *reply;
    __block NSError *error;
    __block BOOL done = NO;
    [device requestSendPTPCommand:command outData:output completion:^(NSData *data, NSData *status, NSError *err) {
        body = data;
        reply = status;
        error = err;
        done = YES;
    }];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:120];
    while (!done && deadline.timeIntervalSinceNow > 0)
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    if (!done) interrupted(@"MTP player timed out");
    if (error) interrupted(error.localizedDescription);
    uint32_t status = number(reply, 6, 2);
    if (result) *result = status;
    else if (status == 0x2007 || status == 0x2003) interrupted(@"MTP session interrupted");
    else if (status != 0x2001) fail([NSString stringWithFormat:@"Player rejected MTP command %04x (%04x)", op, status]);
    if (response) *response = reply;
    return body;
}
static NSData *request(ICCameraDevice *device, uint16_t op, NSArray<NSNumber *> *args,
                       NSData *output, NSData **response) {
    return requestStatus(device, op, args, output, response, NULL);
}
static NSArray *ids(NSData *data) {
    uint32_t count = number(data, 0, 4);
    if (count > (data.length - 4) / 4) fail(@"Invalid MTP handle list");
    NSMutableArray *result = [NSMutableArray arrayWithCapacity:count];
    for (NSUInteger i = 0; i < count; i++) [result addObject:@(number(data, 4+i*4, 4))];
    return result;
}
static NSString *objectName(NSData *info) {
    NSUInteger units = number(info, 52, 1);
    if (!units || 53 + units * 2 > info.length) fail(@"Invalid MTP object name");
    NSString *name = [[NSString alloc] initWithBytes:(const char *)info.bytes + 53 length:(units-1)*2 encoding:NSUTF16LittleEndianStringEncoding];
    if (!name) fail(@"Invalid MTP object name");
    return name;
}
static NSData *encodeName(NSString *name) {
    if (!name.length || name.length > 254) fail(@"Invalid MTP filename length");
    uint8_t count = (uint8_t)(name.length + 1);
    NSMutableData *data = [NSMutableData dataWithBytes:&count length:1];
    [data appendData:[name dataUsingEncoding:NSUTF16LittleEndianStringEncoding]];
    [data increaseLengthBy:2];
    return data;
}
static id execute(ICCameraDevice *device, NSDictionary *command) {
    NSString *op = command[@"op"];
    NSNumber *storage = command[@"storage"], *parent = command[@"parent"], *handle = command[@"id"];
    if ([op isEqual:@"stores"]) return ids(request(device, 0x1004, @[], nil, NULL));
    if ([op isEqual:@"storage"]) {
        NSData *info=request(device,0x1005,@[storage],nil,NULL);
        uint64_t capacity=number(info,6,4) | ((uint64_t)number(info,10,4)<<32);
        uint64_t available=number(info,14,4) | ((uint64_t)number(info,18,4)<<32);
        NSUInteger length=number(info,26,1);
        NSString *name=@"Storage";
        if (length && 27+length*2<=info.length)
            name=[[NSString alloc] initWithBytes:(const char *)info.bytes+27 length:(length-1)*2 encoding:NSUTF16LittleEndianStringEncoding] ?: name;
        return @{@"name":name,@"available":@(available),@"capacity":@(capacity),@"device":device.serialNumberString ?: device.UUIDString ?: @""};
    }
    if ([op isEqual:@"list"]) {
        NSMutableArray *entries = [NSMutableArray array];
        for (NSNumber *child in ids(request(device, 0x1007, @[storage, @0, parent], nil, NULL))) {
            NSData *info = request(device, 0x1008, @[child], nil, NULL);
            [entries addObject:@{@"id":child, @"name":objectName(info), @"size":@(number(info,8,4)),
                                 @"folder":(number(info,4,2)==0x3001 ? @YES : @NO)}];
        }
        return entries;
    }
    if ([op isEqual:@"read"]) {
        NSData *info = request(device, 0x1008, @[handle], nil, NULL);
        if (number(info,8,4) > [command[@"limit"] unsignedLongLongValue]) fail(@"Player file exceeds read limit");
        NSData *data = request(device, 0x1009, @[handle], nil, NULL);
        if (data.length != number(info,8,4)) fail(@"Incomplete MTP file");
        NSError *error;
        if (![data writeToFile:command[@"path"] options:NSDataWritingAtomic error:&error]) fail(error.localizedDescription);
    } else if ([op isEqual:@"create"]) {
        BOOL folder = [command[@"folder"] boolValue];
        NSError *error;
        NSData *data = folder ? nil : [NSData dataWithContentsOfFile:command[@"path"] options:NSDataReadingMappedIfSafe error:&error];
        if (!folder && !data) fail(error.localizedDescription);
        if (data.length >= UINT32_MAX) fail(@"File exceeds MTP transfer size");
        NSMutableData *info = [NSMutableData dataWithLength:52];
        uint8_t *bytes = info.mutableBytes;
        uint32_t sid = storage.unsignedIntValue, pid = parent.unsignedIntValue, size = (uint32_t)data.length;
        uint16_t format = folder ? 0x3001 : 0x3000;
        memcpy(bytes,&sid,4); memcpy(bytes+4,&format,2); memcpy(bytes+8,&size,4);
        memcpy(bytes+38,&pid,4); bytes[42] = folder ? 1 : 0;
        [info appendData:encodeName(command[@"name"])];
        [info increaseLengthBy:3];
        NSData *reply;
        request(device, 0x100c, @[storage,parent], info, &reply);
        uint32_t created = number(reply,20,4);
        if (!created || created == UINT32_MAX) fail(@"Player returned an invalid object handle");
        if (!folder) {
            uint16_t status;
            requestStatus(device, 0x100d, @[], data, NULL, &status);
            if (status == 0x2007)
                interrupted([NSString stringWithFormat:@"Incomplete transfer (%u bytes)", size]);
            if (status != 0x2001)
                fail([NSString stringWithFormat:@"Player rejected file upload (%04x)", status]);
        }
        return @(created);
    } else if ([op isEqual:@"rename"]) {
        request(device,0x9804,@[handle,@0xdc07],encodeName(command[@"name"]),NULL);
    } else if ([op isEqual:@"delete"]) {
        request(device,0x100b,@[handle,@0],nil,NULL);
    } else fail(@"Unknown MTP operation");
    return @YES;
}
@interface MTPWorker : NSObject<ICDeviceBrowserDelegate, ICDeviceDelegate>
@property uint32_t location;
@end
@implementation MTPWorker
-(void)deviceBrowser:(ICDeviceBrowser*)browser didAddDevice:(ICDevice*)device moreComing:(BOOL)more {
    if ((uint32_t)device.usbLocationID == self.location && [device isKindOfClass:ICCameraDevice.class]) {
        device.delegate = self;
        [device requestOpenSession];
    }
}
-(void)deviceBrowser:(ICDeviceBrowser*)b didRemoveDevice:(ICDevice*)d moreGoing:(BOOL)m {}
-(void)didRemoveDevice:(ICDevice*)device { interrupted(@"The player disconnected"); }
-(void)device:(ICDevice*)device didCloseSessionWithError:(NSError*)error { exit(error ? 2 : 0); }
-(void)device:(ICDevice*)device didOpenSessionWithError:(NSError*)error {
    if (error) interrupted(error.localizedDescription);
    [NSTimer scheduledTimerWithTimeInterval:0.01 repeats:NO block:^(NSTimer * __unused timer) {
        char *line = NULL;
        size_t capacity = 0;
        ssize_t length;
        while ((length = getline(&line,&capacity,stdin)) > 0) @autoreleasepool {
            NSDictionary *command = [NSJSONSerialization JSONObjectWithData:[NSData dataWithBytes:line length:length] options:0 error:nil];
            id result = execute((ICCameraDevice*)device, command);
            NSData *json = [NSJSONSerialization dataWithJSONObject:result options:NSJSONWritingFragmentsAllowed error:nil];
            fwrite(json.bytes,1,json.length,stdout);
            fputc('\n',stdout); fflush(stdout);
        }
        free(line);
        [device requestCloseSession];
    }];
}
@end
int textamp_mtp_worker(uint32_t location) {
    @autoreleasepool {
        ICDeviceBrowser *browser = [ICDeviceBrowser new];
        MTPWorker *worker = [MTPWorker new];
        worker.location = location;
        browser.delegate = worker;
        [browser start];
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:10]];
        [browser stop];
        interrupted(@"Timed out opening the MTP session");
        return 2;
    }
}
