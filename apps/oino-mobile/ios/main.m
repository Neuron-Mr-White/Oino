// Oino Mobile iOS host: a full-screen GPUI window driven by CADisplayLink.
//
// GPUI renders everything (conversation, sheets, composer with the software keyboard),
// the same layout as Android (oino_ui::mobile). The display link pauses whenever GPUI
// reports no pending frame and the frame waker resumes it, so an idle app costs no CPU.

#import <UIKit/UIKit.h>
#import <SafariServices/SafariServices.h>
#import <UserNotifications/UserNotifications.h>

#import "oino_ios.h"

@interface OinoAppDelegate : UIResponder <UIApplicationDelegate>
@property(nonatomic, strong) UIWindow *window;
@property(nonatomic, assign) void *gpuiWindow;
@property(nonatomic, strong) CADisplayLink *displayLink;
@end

static void OinoResumeFrames(void *context);

@implementation OinoAppDelegate

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    oino_ios_register_app();
    gpui_ios_run_demo();
    self.gpuiWindow = gpui_ios_get_window();
    if (!self.gpuiWindow) {
        NSLog(@"Oino: GPUI did not create a window");
        return YES;
    }
    [self startDisplayLink];
    return YES;
}

- (void)startDisplayLink {
    if (self.displayLink || !self.gpuiWindow) {
        return;
    }
    self.displayLink = [CADisplayLink displayLinkWithTarget:self selector:@selector(renderFrame)];
    [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
    gpui_ios_set_frame_waker(self.gpuiWindow, OinoResumeFrames, (__bridge void *)self);
}

- (void)stopDisplayLink {
    [self.displayLink invalidate];
    self.displayLink = nil;
}

- (void)renderFrame {
    if (self.gpuiWindow && !gpui_ios_request_frame(self.gpuiWindow)) {
        self.displayLink.paused = YES;
    }
}

static void OinoResumeFrames(void *context) {
    OinoAppDelegate *delegate = (__bridge OinoAppDelegate *)context;
    delegate.displayLink.paused = NO;
}

- (void)applicationWillEnterForeground:(UIApplication *)application {
    gpui_ios_will_enter_foreground(NULL);
    [self startDisplayLink];
}

- (void)applicationDidBecomeActive:(UIApplication *)application {
    gpui_ios_did_become_active(NULL);
}

- (void)applicationWillResignActive:(UIApplication *)application {
    gpui_ios_will_resign_active(NULL);
}

- (void)applicationDidEnterBackground:(UIApplication *)application {
    gpui_ios_did_enter_background(NULL);
    [self stopDisplayLink];
}

- (BOOL)application:(UIApplication *)application
            openURL:(NSURL *)url
            options:(NSDictionary<UIApplicationOpenURLOptionsKey, id> *)options {
    gpui_ios_handle_open_url((__bridge void *)[url absoluteString]);
    return YES;
}

- (void)applicationWillTerminate:(UIApplication *)application {
    [self stopDisplayLink];
    gpui_ios_will_terminate(NULL);
}

@end

// In-app browser (design.md A1.4): SFSafariViewController over the GPUI window.
bool oino_ios_open_browser(const char *url, bool dark) {
    if (!url) {
        return false;
    }
    NSURL *nsurl = [NSURL URLWithString:[NSString stringWithUTF8String:url]];
    if (!nsurl || !([nsurl.scheme isEqualToString:@"https"] || [nsurl.scheme isEqualToString:@"http"])) {
        return false;
    }
    dispatch_async(dispatch_get_main_queue(), ^{
        // GPUI owns the key window (not the delegate's).
        UIWindow *key = nil;
        for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
            if (![scene isKindOfClass:UIWindowScene.class]) {
                continue;
            }
            for (UIWindow *w in ((UIWindowScene *)scene).windows) {
                if (w.isKeyWindow || !key) {
                    key = w;
                }
            }
        }
        UIViewController *root = key.rootViewController;
        while (root.presentedViewController) {
            root = root.presentedViewController;
        }
        if (!root) {
            [UIApplication.sharedApplication openURL:nsurl options:@{} completionHandler:nil];
            return;
        }
        SFSafariViewController *safari = [[SFSafariViewController alloc] initWithURL:nsurl];
        safari.overrideUserInterfaceStyle = dark ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
        safari.preferredControlTintColor = [UIColor colorWithRed:0xF0 / 255.0 green:0x30 / 255.0 blue:0x18 / 255.0 alpha:1];
        [root presentViewController:safari animated:YES completion:nil];
    });
    return true;
}

// App icon badge = sessions that need you (design.md §6.11).
void oino_ios_set_badge(long count) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (@available(iOS 16.0, *)) {
            [[UNUserNotificationCenter currentNotificationCenter] setBadgeCount:count withCompletionHandler:nil];
        } else {
            UIApplication.sharedApplication.applicationIconBadgeNumber = count;
        }
    });
}

int main(int argc, char *argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass([OinoAppDelegate class]));
    }
}
