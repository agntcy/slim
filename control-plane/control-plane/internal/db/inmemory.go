package db

import (
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

type dbService struct {
	nodes    map[string]Node
	routes   map[string]Route
	channels map[string]Channel
	mu       sync.RWMutex
}

func NewInMemoryDBService() DataAccess {
	return &dbService{
		nodes:    make(map[string]Node),
		routes:   make(map[string]Route, 100),
		channels: make(map[string]Channel),
	}
}

func (d *dbService) SaveChannel(channelID string, moderators []string) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if _, exists := d.channels[channelID]; exists {
		return fmt.Errorf("channel with ID %s already exists", channelID)
	}

	d.channels[channelID] = Channel{
		ID:           channelID,
		Moderators:   moderators,
		Participants: make([]string, 0),
	}
	return nil
}

func (d *dbService) DeleteChannel(channelID string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	if _, exists := d.channels[channelID]; !exists {
		return fmt.Errorf("channel with ID %s not found", channelID)
	}

	delete(d.channels, channelID)
	return nil
}

func (d *dbService) GetChannel(channelID string) (Channel, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	channel, exists := d.channels[channelID]
	if !exists {
		return Channel{}, fmt.Errorf("channel with ID %s not found", channelID)
	}
	return channel, nil
}

func (d *dbService) UpdateChannel(channel Channel) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if channel.ID == "" {
		return fmt.Errorf("channel ID cannot be empty")
	}

	if _, exists := d.channels[channel.ID]; !exists {
		return fmt.Errorf("channel with ID %s not found", channel.ID)
	}

	d.channels[channel.ID] = channel
	return nil
}

func (d *dbService) ListChannels() ([]Channel, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	channels := make([]Channel, 0, len(d.channels))
	for _, channel := range d.channels {
		channels = append(channels, channel)
	}
	return channels, nil
}

func (d *dbService) ListNodes() []Node {
	d.mu.RLock()
	defer d.mu.RUnlock()

	nodes := make([]Node, 0, len(d.nodes))
	for _, node := range d.nodes {
		nodes = append(nodes, node)
	}

	return nodes
}

func (d *dbService) GetNode(id string) (*Node, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	node, exists := d.nodes[id]
	if !exists {
		return nil, fmt.Errorf("node with ID %s not found", id)
	}

	return &node, nil
}

// SaveNode implements DataAccess.
func (d *dbService) SaveNode(node Node) (string, bool, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	connDetailsChanged := false
	if node.ID == "" {
		node.ID = uuid.New().String()
		connDetailsChanged = true // New node means connection details are new
	} else {
		// Check if node exists and compare connection details
		if existingNode, exists := d.nodes[node.ID]; exists {
			connDetailsChanged = d.hasConnectionDetailsChanged(existingNode.ConnDetails, node.ConnDetails)
		} else {
			connDetailsChanged = true // Node ID provided but doesn't exist, treat as new
		}
	}

	node.LastUpdated = time.Now()
	d.nodes[node.ID] = node

	return node.ID, connDetailsChanged, nil
}

// hasConnectionDetailsChanged compares two slices of ConnectionDetails and returns true if they differ
func (d *dbService) hasConnectionDetailsChanged(existing, newConnDetails []ConnectionDetails) bool {
	// If lengths are different, details have changed
	if len(existing) != len(newConnDetails) {
		return true
	}

	// Create maps for efficient comparison
	existingMap := make(map[string]ConnectionDetails)
	for _, cd := range existing {
		key := d.getConnectionDetailsKey(cd)
		existingMap[key] = cd
	}

	newMap := make(map[string]ConnectionDetails)
	for _, cd := range newConnDetails {
		key := d.getConnectionDetailsKey(cd)
		newMap[key] = cd
	}

	// Check if all existing connections are still present and unchanged
	for key, existingCD := range existingMap {
		newCD, exists := newMap[key]
		if !exists {
			return true // Connection removed
		}
		if !d.connectionDetailsEqual(existingCD, newCD) {
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
func (d *dbService) getConnectionDetailsKey(cd ConnectionDetails) string {
	return cd.Endpoint // Use endpoint as the primary key
}

// connectionDetailsEqual compares two ConnectionDetails structs for equality
func (d *dbService) connectionDetailsEqual(cd1, cd2 ConnectionDetails) bool {
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

	// Compare GroupName (handling nil pointers)
	if cd1.GroupName == nil && cd2.GroupName != nil {
		return false
	}
	if cd1.GroupName != nil && cd2.GroupName == nil {
		return false
	}
	if cd1.GroupName != nil && cd2.GroupName != nil {
		if *cd1.GroupName != *cd2.GroupName {
			return false
		}
	}

	return true
}

// DeleteNode implements DataAccess.
func (d *dbService) DeleteNode(id string) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if _, exists := d.nodes[id]; !exists {
		return fmt.Errorf("node with ID %s not found", id)
	}

	delete(d.nodes, id)

	return nil
}

func (d *dbService) AddRoute(route Route) string {
	routeID := route.GetID()
	// Add route to the map
	d.mu.Lock()
	defer d.mu.Unlock()
	route.LastUpdated = time.Now()
	d.routes[routeID] = route
	return routeID
}

func (d *dbService) DeleteRoute(routeID string) error {
	// Remove route from the map
	d.mu.Lock()
	defer d.mu.Unlock()
	if _, exists := d.routes[routeID]; !exists {
		return fmt.Errorf("route %s not found", routeID)
	}
	delete(d.routes, routeID)
	return nil
}

func (d *dbService) MarkRouteAsDeleted(routeID string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %s not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Deleted = true
	d.routes[routeID] = route
	return nil
}

// MarkRouteAsApplied implements DataAccess.
func (d *dbService) MarkRouteAsApplied(routeID string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %s not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Applied = true
	route.FailedMsg = ""
	d.routes[routeID] = route
	return nil
}

// MarkRouteAsFailed implements DataAccess.
func (d *dbService) MarkRouteAsFailed(routeID string, msg string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %s not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Applied = false
	route.FailedMsg = msg
	d.routes[routeID] = route
	return nil
}

func (d *dbService) GetRouteByID(routeID string) *Route {
	d.mu.RLock()
	defer d.mu.RUnlock()
	route := d.routes[routeID]
	return &route
}

func (d *dbService) GetRoutesForNodeID(nodeID string) []Route {
	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		if route.SourceNodeID == nodeID {
			routes = append(routes, route)
		}
	}

	// Return empty slice instead of error when no routes found
	return routes
}

func (d *dbService) GetRoutesForDestinationNodeID(nodeID string) []Route {
	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		if route.DestNodeID == nodeID {
			routes = append(routes, route)
		}
	}

	// Return empty slice instead of error when no routes found
	return routes
}

func (d *dbService) GetRoutesForDestinationNodeIDAndName(nodeID string, Component0 string, Component1 string,
	Component2 string, ComponentID *wrapperspb.UInt64Value) []Route {

	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		if route.DestNodeID == nodeID &&
			route.Component0 == Component0 &&
			route.Component1 == Component1 &&
			route.Component2 == Component2 {
			if (ComponentID == nil && route.ComponentID == nil) ||
				(ComponentID != nil && route.ComponentID != nil && ComponentID.Value == route.ComponentID.Value) {
				routes = append(routes, route)
			}
		}
	}
	return routes
}
