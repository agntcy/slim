#import "AgntcySlimBindings.h"
#import <React/RCTBridge+Private.h>
#import <React/RCTUtils.h>
#import "../generated/cpp/slim_bindings.hpp"

#ifdef RCT_NEW_ARCH_ENABLED
#import <React-RCTAppDelegate/RCTAppDelegate.h>
#import <ReactCommon/RCTTurboModuleManager.h>
#else
#import <React/RCTBridge.h>
#endif

#ifdef __cplusplus
#import "jsi/jsi.h"
using namespace facebook;
#endif

// Declare the private runtime property on RCTCxxBridge
@interface RCTCxxBridge (JSIRuntime)
- (void *)runtime;
@end

@implementation AgntcySlimBindings

@synthesize bridge = _bridge;
@synthesize methodQueue = _methodQueue;

RCT_EXPORT_MODULE(SlimBindings)

+ (BOOL)requiresMainQueueSetup
{
  return YES;
}

- (void)setBridge:(RCTBridge *)bridge
{
  _bridge = bridge;
  RCTCxxBridge *cxxBridge = (RCTCxxBridge *)bridge;
  
  if (!cxxBridge) {
    NSLog(@"❌ AgntcySlimBindings: Bridge is not a CxxBridge");
    return;
  }
  
  NSLog(@"🔧 AgntcySlimBindings: setBridge called, will install JSI bindings");
  
  // Install JSI bindings - need to wait for JS runtime to be ready
  RCTExecuteOnMainQueue(^{
    [self installJSIBindingsWithBridge:cxxBridge];
  });
}

- (void)installJSIBindingsWithBridge:(RCTCxxBridge *)cxxBridge
{
  NSLog(@"🚀 AgntcySlimBindings: Installing JSI bindings...");
  
  // Get the JSI runtime directly
  void *runtimePtr = [cxxBridge runtime];
  if (!runtimePtr) {
    NSLog(@"❌ AgntcySlimBindings: Runtime pointer is NULL - JS bundle may not be loaded yet");
    // Retry after a short delay
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.1 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
      [self installJSIBindingsWithBridge:cxxBridge];
    });
    return;
  }
  
  facebook::jsi::Runtime *runtime = (facebook::jsi::Runtime *)runtimePtr;
  std::shared_ptr<facebook::react::CallInvoker> callInvoker = cxxBridge.jsCallInvoker;
  
  if (!callInvoker) {
    NSLog(@"❌ AgntcySlimBindings: CallInvoker is NULL");
    return;
  }
  
  NSLog(@"🔧 AgntcySlimBindings: Runtime and CallInvoker obtained, registering module");
  
  try {
    // Register the JSI module
    NativeSlimBindings::registerModule(*runtime, callInvoker);
    NSLog(@"✅ AgntcySlimBindings: JSI module registered successfully!");
  } catch (const std::exception &e) {
    NSLog(@"❌ AgntcySlimBindings: Exception during registration: %s", e.what());
  } catch (...) {
    NSLog(@"❌ AgntcySlimBindings: Unknown exception during registration");
  }
}

RCT_EXPORT_BLOCKING_SYNCHRONOUS_METHOD(install)
{
  NSLog(@"📞 AgntcySlimBindings: install() called (JSI should already be installed)");
  return @YES;
}

@end
