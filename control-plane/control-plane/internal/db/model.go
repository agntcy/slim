package db

import (
	"fmt"
	"hash/fnv"
	"strings"
	"time"

	"google.golang.org/protobuf/types/known/wrapperspb"
)

type Node struct {
	ID          string
	GroupName   *string
	ConnDetails []ConnectionDetails
	LastUpdated time.Time
}

type ConnectionDetails struct {
	Endpoint         string
	ExternalEndpoint *string
	TrustDomain      *string
	MTLSRequired     bool
	TLSConfig        *SeverTLSConfig
	AuthConfig       *Auth
}

func (cd ConnectionDetails) String() string {
	parts := []string{fmt.Sprintf("endpoint: %s", cd.Endpoint)}
	if cd.MTLSRequired {
		parts = append(parts, "mtls")
	}
	if cd.ExternalEndpoint != nil && *cd.ExternalEndpoint != "" {
		parts = append(parts, fmt.Sprintf("externalEndpoint: %s", *cd.ExternalEndpoint))
	}
	if cd.TrustDomain != nil && *cd.TrustDomain != "" {
		parts = append(parts, fmt.Sprintf("trustDomain: %s", *cd.TrustDomain))
	}
	return strings.Join(parts, ", ")
}

// hasConnectionDetailsChanged compares two slices of ConnectionDetails and returns true if they differ
func hasConnectionDetailsChanged(existing, newConnDetails []ConnectionDetails) bool {
	// If lengths are different, details have changed
	if len(existing) != len(newConnDetails) {
		return true
	}

	// Create maps for efficient comparison
	existingMap := make(map[string]ConnectionDetails)
	for _, cd := range existing {
		key := getConnectionDetailsKey(cd)
		existingMap[key] = cd
	}

	newMap := make(map[string]ConnectionDetails)
	for _, cd := range newConnDetails {
		key := getConnectionDetailsKey(cd)
		newMap[key] = cd
	}

	// Check if all existing connections are still present and unchanged
	for key, existingCD := range existingMap {
		newCD, exists := newMap[key]
		if !exists {
			return true // Connection removed
		}
		if !connectionDetailsEqual(existingCD, newCD) {
			return true // Connection details changed
		}
	}

	// Check if there are any new connections
	for key := range newMap {
		if _, exists := existingMap[key]; !exists {
			return true // New connection added
		}
	}

	return false
}

// getConnectionDetailsKey creates a unique key for a ConnectionDetails for comparison
func getConnectionDetailsKey(cd ConnectionDetails) string {
	return cd.Endpoint // Use endpoint as the primary key
}

// connectionDetailsEqual compares two ConnectionDetails structs for equality
func connectionDetailsEqual(cd1, cd2 ConnectionDetails) bool {
	// Compare Endpoint
	if cd1.Endpoint != cd2.Endpoint {
		return false
	}

	// Compare MTLSRequired
	if cd1.MTLSRequired != cd2.MTLSRequired {
		return false
	}

	// Compare ExternalEndpoint (handling nil pointers)
	if cd1.ExternalEndpoint == nil && cd2.ExternalEndpoint != nil {
		return false
	}
	if cd1.ExternalEndpoint != nil && cd2.ExternalEndpoint == nil {
		return false
	}
	if cd1.ExternalEndpoint != nil && cd2.ExternalEndpoint != nil {
		if *cd1.ExternalEndpoint != *cd2.ExternalEndpoint {
			return false
		}
	}

	return true
}

// RouteStatus represents the status of a route.
type RouteStatus int

const (
	RouteStatusApplied RouteStatus = iota
	RouteStatusFailed
)

type Route struct {
	ID uint64
	// ID of the node which the route is applied to.
	// If SourceNodeID is AllNodesID, the route applies to all nodes
	SourceNodeID string
	// if DestNodeID is empty, DestEndpoint should be used to determine the destination
	DestNodeID   string
	DestEndpoint string
	// ConnConfigData is a JSON string containing connection configuration details in case DestEndpoint is set
	ConnConfigData string
	Component0     string
	Component1     string
	Component2     string
	ComponentID    *wrapperspb.UInt64Value
	Status         RouteStatus
	StatusMsg      string
	Deleted        bool
	LastUpdated    time.Time
}

func (r Route) GetUniqueID() uint64 {
	// Use a separator that is unlikely to appear in the actual strings
	// to prevent collision issues like "a" + "bc" vs "ab" + "c".
	separator := "\x00" // Null character is a good choice as it's often not in text data.

	compID := ""
	if r.ComponentID != nil {
		compID = fmt.Sprintf("%d", r.ComponentID.Value)
	}
	// Concatenate all strings with the separator
	combinedString := strings.Join([]string{r.SourceNodeID, r.Component0, r.Component1,
		r.Component2, compID, r.DestNodeID, r.DestEndpoint}, separator)

	// Create a new FNV-1a 64-bit hash.
	// FNV-1a is a non-cryptographic hash function suitable for hash table lookups.
	h := fnv.New64a()

	// Write the combined string bytes to the hash function.
	// The Write method never returns an error for fnv.New64a().
	h.Write([]byte(combinedString))

	// Sum64 returns the 64-bit hash value.
	// Ensure the high bit is not set to avoid SQLite issues
	id := h.Sum64() & 0x7FFFFFFFFFFFFFFF
	return id
}

func (r Route) String() string {
	return fmt.Sprintf("%s:%s/%s/%s/%v->%s[%s]", r.SourceNodeID,
		r.Component0, r.Component1, r.Component2, r.ComponentID, r.DestNodeID, r.DestEndpoint)
}

type Channel struct {
	ID           string
	Moderators   []string
	Participants []string
}
