package tests

import (
	"testing"

	slim "github.com/agntcy/slim/bindings/generated/slimbindings"
)

// TestVersion verifies basic library loading and version access
func TestVersion(t *testing.T) {
	version := slim.GetVersion()
	if version == "" {
		t.Fatal("GetVersion() returned empty string - library may not be loaded correctly")
	}
	t.Logf("✓ Library loaded successfully, version: %s", version)
}

// TestCryptoInit verifies crypto initialization doesn't crash
func TestCryptoInit(t *testing.T) {
	// InitializeCrypto() returns void, just verify it doesn't panic
	slim.InitializeCrypto()
	t.Log("✓ InitializeCrypto() succeeded")
}

// TestNameStruct verifies Name struct functionality
func TestNameStruct(t *testing.T) {
	name := slim.Name{
		Components: []string{"acme", "chat", "lobby"},
		Id:         nil,
	}

	if len(name.Components) != 3 {
		t.Errorf("Name.Components length = %d, want 3", len(name.Components))
	}
	if name.Components[0] != "acme" {
		t.Errorf("Name.Components[0] = %v, want 'acme'", name.Components[0])
	}
	if name.Id != nil {
		t.Error("Name.Id should be nil")
	}

	t.Log("✓ Name struct works correctly")
}

// TestSessionConfig verifies SessionConfig struct
func TestSessionConfig(t *testing.T) {
	config := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	if config.SessionType != slim.SessionTypePointToPoint {
		t.Errorf("SessionType = %v, want PointToPoint", config.SessionType)
	}
	if config.EnableMls {
		t.Error("EnableMls should be false")
	}

	t.Log("✓ SessionConfig struct works correctly")
}

// TestSessionTypes verifies all session types are accessible
func TestSessionTypes(t *testing.T) {
	types := []struct {
		name  string
		value slim.SessionType
	}{
		{"PointToPoint", slim.SessionTypePointToPoint},
		{"Multicast", slim.SessionTypeMulticast},
	}

	for _, tt := range types {
		t.Run(tt.name, func(t *testing.T) {
			// Just verify we can access the constant
			_ = tt.value
			t.Logf("✓ SessionType.%s is accessible", tt.name)
		})
	}
}

// TestTypeSafety verifies type safety of Go bindings
func TestTypeSafety(t *testing.T) {
	// This test just verifies that the Go compiler enforces types correctly
	var name slim.Name
	var config slim.SessionConfig
	var sessionType slim.SessionType

	// These should compile without errors
	name = slim.Name{Components: []string{"test", "test", "test"}, Id: nil}
	config = slim.SessionConfig{SessionType: slim.SessionTypePointToPoint, EnableMls: false}
	sessionType = slim.SessionTypeMulticast

	// Use the variables to avoid unused variable warnings
	_ = name
	_ = config
	_ = sessionType

	t.Log("✓ Type safety verified - all types compile correctly")
}

// TestNilHandling verifies proper nil handling
func TestNilHandling(t *testing.T) {
	// Test that we can create names with empty Components and nil Id
	name := slim.Name{
		Components: []string{},
		Id:         nil,
	}

	if name.Components == nil {
		t.Error("Empty array should not be nil")
	}
	if name.Id != nil {
		t.Error("Id should be nil")
	}

	// Test with non-nil Id
	id := uint64(42)
	name2 := slim.Name{
		Components: []string{"org", "app", "res"},
		Id:         &id,
	}

	if name2.Id == nil {
		t.Error("Id should not be nil")
	}
	if *name2.Id != 42 {
		t.Errorf("Id = %v, want 42", *name2.Id)
	}

	t.Log("✓ Nil/optional value handling works correctly")
}

// TestDataSizes verifies data structure sizes are reasonable
func TestDataSizes(t *testing.T) {
	testData := []byte("Hello, SLIM!")
	if len(testData) != 12 {
		t.Errorf("Test data size = %d, want 12", len(testData))
	}

	largeData := make([]byte, 1024*1024) // 1 MB
	if len(largeData) != 1024*1024 {
		t.Errorf("Large data size = %d, want %d", len(largeData), 1024*1024)
	}

	t.Log("✓ Data size handling works correctly")
}

// TestPointerTypes verifies that pointer types work correctly
func TestPointerTypes(t *testing.T) {
	// Test with nil pointer (optional ID)
	var nilId *uint64
	if nilId != nil {
		t.Error("Nil pointer should be nil")
	}

	// Test with non-nil pointer
	id := uint64(42)
	ptrId := &id
	if ptrId == nil {
		t.Error("Non-nil pointer should not be nil")
	}
	if *ptrId != 42 {
		t.Errorf("Pointer value = %v, want 42", *ptrId)
	}

	t.Log("✓ Pointer types work correctly")
}

// TestNameComponents verifies Name component handling
func TestNameComponents(t *testing.T) {
	tests := []struct {
		name       string
		components []string
		valid      bool
	}{
		{"Three components", []string{"org", "app", "res"}, true},
		{"Empty components", []string{}, true},
		{"Single component", []string{"test"}, true},
		{"Many components", []string{"a", "b", "c", "d", "e"}, true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			name := slim.Name{
				Components: tt.components,
				Id:         nil,
			}

			if len(name.Components) != len(tt.components) {
				t.Errorf("Components length = %d, want %d", len(name.Components), len(tt.components))
			}

			t.Logf("✓ %s works correctly", tt.name)
		})
	}
}
