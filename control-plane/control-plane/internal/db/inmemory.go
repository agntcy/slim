package db

import (
	"fmt"
	"sync"

	"github.com/google/uuid"
)

type dbService struct {
	nodes         map[string]Node
	routes        map[string]Route
	connections   map[string]Connection
	subscriptions map[string]Subscription
	channels      map[string]Channel
	mu            sync.RWMutex
}

const AllNodesID = "*"

func NewInMemoryDBService() DataAccess {
	return &dbService{
		nodes:         make(map[string]Node),
		routes:        make(map[string]Route, 100),
		connections:   make(map[string]Connection),
		subscriptions: make(map[string]Subscription),
		channels:      make(map[string]Channel),
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

func (d *dbService) ListNodes() ([]Node, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	nodes := make([]Node, 0, len(d.nodes))
	for _, node := range d.nodes {
		nodes = append(nodes, node)
	}

	return nodes, nil
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

// GetConnectionDetails implements DataAccess.
func (d *dbService) GetConnection(connectionID string) (Connection, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	connection, exists := d.connections[connectionID]
	if !exists {
		return Connection{}, fmt.Errorf("connection with ID %s not found", connectionID)
	}
	_, exists = d.nodes[connection.NodeID]
	if !exists {
		return Connection{}, fmt.Errorf("node with ID %s not found for connection %s", connection.NodeID, connectionID)
	}

	return connection, nil
}

// GetSubscriptionDetails implements DataAccess.
func (d *dbService) GetSubscription(subscriptionID string) (Subscription, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	subscription, exists := d.subscriptions[subscriptionID]
	if !exists {
		return Subscription{}, fmt.Errorf("subscription with ID %s not found", subscriptionID)
	}

	return subscription, nil
}

// SaveNode implements DataAccess.
func (d *dbService) SaveNode(node Node) (string, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	if node.ID == "" {
		node.ID = uuid.New().String()
	}

	d.nodes[node.ID] = node
	return node.ID, nil
}

// SaveConnection implements DataAccess.
func (d *dbService) SaveConnection(connection Connection) (string, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	// Verify that the node exists
	if _, exists := d.nodes[connection.NodeID]; !exists {
		return "", fmt.Errorf("node with ID %s not found", connection.NodeID)
	}

	if connection.ID == "" {
		connection.ID = uuid.New().String()
	}

	d.connections[connection.ID] = connection
	return connection.ID, nil
}

// SaveSubscription implements DataAccess.
func (d *dbService) SaveSubscription(subscription Subscription) (string, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	// Verify that the node exists
	if _, exists := d.nodes[subscription.NodeID]; !exists {
		return "", fmt.Errorf("node with ID %s not found", subscription.NodeID)
	}

	// Verify that the connection exists
	if _, exists := d.connections[subscription.ConnectionID]; !exists {
		return "", fmt.Errorf("connection with ID %s not found", subscription.ConnectionID)
	}

	if subscription.ID == "" {
		subscription.ID = uuid.New().String()
	}

	d.subscriptions[subscription.ID] = subscription
	return subscription.ID, nil
}

// DeleteConnection implements DataAccess.
func (d *dbService) DeleteConnection(connectionID string) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if _, exists := d.connections[connectionID]; !exists {
		return fmt.Errorf("connection with ID %s not found", connectionID)
	}

	delete(d.connections, connectionID)
	return nil
}

// DeleteSubscription implements DataAccess.
func (d *dbService) DeleteSubscription(subscriptionID string) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if _, exists := d.subscriptions[subscriptionID]; !exists {
		return fmt.Errorf("subscription with ID %s not found", subscriptionID)
	}

	delete(d.subscriptions, subscriptionID)
	return nil
}

// DeleteNode implements DataAccess.
func (d *dbService) DeleteNode(id string) error {
	d.mu.Lock()
	defer d.mu.Unlock()

	if _, exists := d.nodes[id]; !exists {
		return fmt.Errorf("node with ID %s not found", id)
	}

	delete(d.nodes, id)

	// Remove all connections and subscriptions associated with this node
	for connectionID, connection := range d.connections {
		if connection.NodeID == id {
			delete(d.connections, connectionID)
		}
	}

	for subscriptionID, subscription := range d.subscriptions {
		if subscription.NodeID == id {
			delete(d.subscriptions, subscriptionID)
		}
	}

	return nil
}

// ListConnectionsByNodeID implements DataAccess.
func (d *dbService) ListConnectionsByNodeID(nodeID string) ([]Connection, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	if _, exists := d.nodes[nodeID]; !exists {
		return nil, fmt.Errorf("node with ID %s not found", nodeID)
	}

	var connections []Connection
	for _, connection := range d.connections {
		if connection.NodeID == nodeID {
			connections = append(connections, connection)
		}
	}

	// Return empty slice instead of error when no connections found
	return connections, nil
}

// ListSubscriptionsByNodeID implements DataAccess.
func (d *dbService) ListSubscriptionsByNodeID(nodeID string) ([]Subscription, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	if _, exists := d.nodes[nodeID]; !exists {
		return nil, fmt.Errorf("node with ID %s not found", nodeID)
	}

	var subscriptions []Subscription
	for _, subscription := range d.subscriptions {
		if subscription.NodeID == nodeID {
			subscriptions = append(subscriptions, subscription)
		}
	}

	// Return empty slice instead of error when no subscriptions found
	return subscriptions, nil
}

func calculateRouteID(route Route) string {
	return fmt.Sprintf("%s:%s/%s/%s/%v->%s", route.SourceNodeID,
		route.Component0, route.Component1, route.Component2, route.ComponentID, route.DestNodeID)
}

func (d *dbService) AddRoute(route Route) error {
	routeID := calculateRouteID(route)
	// Add route to the map
	d.mu.Lock()
	defer d.mu.Unlock()
	d.routes[routeID] = route
	return nil
}

func (d *dbService) DeleteRoute(route Route) error {
	routeID := calculateRouteID(route)
	// Remove route from the map
	d.mu.Lock()
	defer d.mu.Unlock()
	if _, exists := d.routes[routeID]; !exists {
		return fmt.Errorf("route %s not found", routeID)
	}
	delete(d.routes, routeID)
	return nil
}

func (d *dbService) GetRoutesForNodeID(nodeID string) ([]Route, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		if route.SourceNodeID == nodeID || (route.SourceNodeID == AllNodesID && route.DestNodeID != nodeID) {
			routes = append(routes, route)
		}
	}

	// Return empty slice instead of error when no routes found
	return routes, nil
}
