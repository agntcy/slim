package db

import (
	"fmt"
	"sync"

	"github.com/google/uuid"
)

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

type dbService struct {
	nodes         map[string]Node
	connections   map[string]Connection
	subscriptions map[string]Subscription
	mu            sync.RWMutex
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

	if len(connections) == 0 {
		return nil, fmt.Errorf("no connections found for node ID %s", nodeID)
	}

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

	if len(subscriptions) == 0 {
		return nil, fmt.Errorf("no subscriptions found for node ID %s", nodeID)
	}

	return subscriptions, nil
}

func NewInMemoryDBService() DataAccess {
	return &dbService{
		nodes:         make(map[string]Node),
		connections:   make(map[string]Connection),
		subscriptions: make(map[string]Subscription),
	}
}
