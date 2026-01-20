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
  
  # Link against the Rust library
  s.vendored_libraries = "../../target/aarch64-apple-ios-sim/debug/libslim_bindings.a"
  
  # System frameworks required by the Rust library
  s.frameworks = 'Security', 'CoreFoundation'
  s.libraries = 'c++', 'resolv'
  
  # Required for JSI
  s.pod_target_xcconfig = {
    "USE_HEADERMAP" => "YES",
    "HEADER_SEARCH_PATHS" => "\"$(PODS_ROOT)/Headers/Public/React-Codegen/react/renderer/components\" \"$(PODS_TARGET_SRCROOT)/generated/cpp\" \"$(PODS_TARGET_SRCROOT)/node_modules/uniffi-bindgen-react-native/cpp/includes\"",
    "EXCLUDED_ARCHS[sdk=iphonesimulator*]" => "x86_64"
  }
  
  # Force load the Rust library to ensure all UniFFI symbols are included
  s.xcconfig = {
    "OTHER_LDFLAGS" => "-force_load \"$(PODS_ROOT)/../../../../../../../target/aarch64-apple-ios-sim/debug/libslim_bindings.a\""
  }

  install_modules_dependencies(s)
end
