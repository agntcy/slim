package db

import (
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
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
func (d *dbService) SaveNode(node Node) (string, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	if node.ID == "" {
		node.ID = uuid.New().String()
	}

	node.LastUpdated = time.Now()

	d.nodes[node.ID] = node
	return node.ID, nil
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
