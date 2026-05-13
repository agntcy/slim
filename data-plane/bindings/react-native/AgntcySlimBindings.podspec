require "json"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))

Pod::Spec.new do |s|
  s.name         = "AgntcySlimBindings"
  s.version      = package["version"]
  s.summary      = package["description"]
  s.homepage     = "https://github.com/agntcy/slim"
  s.license      = package["license"]
  s.authors      = package["author"]

  s.platforms    = { :ios => "13.4" }
  s.source       = { :git => "https://github.com/agntcy/slim.git", :tag => "#{s.version}" }

  # Only include JSI bindings, NOT Node.js N-API files
  s.source_files = "generated/cpp/slim_bindings.{cpp,hpp}", "ios/*.{h,mm}"
  s.exclude_files = "generated/cpp/node_module.cpp"
  
  # Public headers for Objective-C
  s.public_header_files = "ios/*.h"
  
  # Link against the Rust library (XCFramework: device + simulator slices for npm publish)
  s.vendored_frameworks = "$(PODS_TARGET_SRCROOT)/ios/SlimBindings.xcframework"

  # Required for CocoaPods to properly link the vendored XCFramework static libs
  # when using use_frameworks! :linkage => :static (React Native default)
  s.static_framework = true

  # System frameworks required by the Rust library
  s.frameworks = 'Security', 'CoreFoundation'
  s.libraries = 'c++', 'resolv'

  # Required for JSI
  s.dependency "React-Core"
  s.dependency "React-callinvoker"
  s.dependency "React-jsi"

  s.pod_target_xcconfig = {
    "USE_HEADERMAP" => "YES",
    "HEADER_SEARCH_PATHS" => "\"$(PODS_ROOT)/Headers/Public/React-Codegen/react/renderer/components\" \"$(PODS_TARGET_SRCROOT)/generated/cpp\" \"$(PODS_TARGET_SRCROOT)/node_modules/uniffi-bindgen-react-native/cpp/includes\" \"$(PODS_ROOT)/Headers/Private/React-Core\" \"$(PODS_ROOT)/Headers/Public/React-jsi\"",
    "EXCLUDED_ARCHS[sdk=iphonesimulator*]" => "x86_64",
    "CLANG_CXX_LANGUAGE_STANDARD" => "c++20",
    # Explicitly link libslim_bindings from XCFramework - CocoaPods may not add vendored XCFramework
    # static libs to the Frameworks phase, so we add search paths and -lslim_bindings
    "LIBRARY_SEARCH_PATHS[sdk=iphoneos*]" => "$(inherited) \"$(PODS_TARGET_SRCROOT)/ios/SlimBindings.xcframework/ios-arm64\"",
    "LIBRARY_SEARCH_PATHS[sdk=iphonesimulator*]" => "$(inherited) \"$(PODS_TARGET_SRCROOT)/ios/SlimBindings.xcframework/ios-arm64-simulator\"",
    "OTHER_LDFLAGS" => "$(inherited) -lslim_bindings"
  }

  # Propagate libslim_bindings to the app target (fixes undefined symbol: ffi_slim_bindings_*)
  # PODS_TARGET_SRCROOT works in user_target_xcconfig when the pod is a development pod
  s.user_target_xcconfig = {
    "LIBRARY_SEARCH_PATHS[sdk=iphoneos*]" => "$(inherited) \"$(PODS_TARGET_SRCROOT)/ios/SlimBindings.xcframework/ios-arm64\"",
    "LIBRARY_SEARCH_PATHS[sdk=iphonesimulator*]" => "$(inherited) \"$(PODS_TARGET_SRCROOT)/ios/SlimBindings.xcframework/ios-arm64-simulator\"",
    "OTHER_LDFLAGS" => "$(inherited) -lslim_bindings"
  }

  install_modules_dependencies(s)
end
