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
	routes   map[uint64]Route
	links    map[string]Link
	channels map[string]Channel
	mu       sync.RWMutex
}

func linkStorageKey(link Link) string {
	return fmt.Sprintf("%s|%s|%s|%s", link.SourceNodeID, link.DestNodeID, link.DestEndpoint, link.LinkID)
}

func NewInMemoryDBService() DataAccess {
	return &dbService{
		nodes:    make(map[string]Node),
		routes:   make(map[uint64]Route, 100),
		links:    make(map[string]Link, 100),
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
		connDetailsChanged = false
	} else {
		// Check if node exists and compare connection details
		if existingNode, exists := d.nodes[node.ID]; exists {
			connDetailsChanged = hasConnectionDetailsChanged(existingNode.ConnDetails, node.ConnDetails)
		} else {
			connDetailsChanged = false
		}
	}

	node.LastUpdated = time.Now()
	d.nodes[node.ID] = node

	return node.ID, connDetailsChanged, nil
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

func (d *dbService) AddRoute(route Route) (Route, error) {
	routeID := route.GetUniqueID()
	d.mu.Lock()
	defer d.mu.Unlock()
	if _, exists := d.routes[routeID]; exists {
		return Route{}, fmt.Errorf("route %s already exists", route.String())
	}

	// Add route to the map
	route.ID = routeID
	route.LastUpdated = time.Now()
	d.routes[routeID] = route
	return route, nil
}

func (d *dbService) AddLink(link Link) (Link, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if link.SourceNodeID == "" || link.DestNodeID == "" || link.DestEndpoint == "" {
		return Link{}, fmt.Errorf("sourceNodeID, destNodeID and destEndpoint are required")
	}
	for _, existing := range d.links {
		if !existing.Deleted &&
			existing.SourceNodeID == link.SourceNodeID &&
			existing.DestEndpoint == link.DestEndpoint {
			link.LinkID = existing.LinkID
			break
		}
	}
	if link.LinkID == "" {
		link.LinkID = uuid.NewString()
	}
	link.LastUpdated = time.Now()
	d.links[linkStorageKey(link)] = link
	return link, nil
}

func (d *dbService) UpdateLink(link Link) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	key := linkStorageKey(link)
	if link.LinkID == "" || link.SourceNodeID == "" || link.DestNodeID == "" || link.DestEndpoint == "" {
		return fmt.Errorf("link identity fields cannot be empty")
	}
	if _, exists := d.links[key]; !exists {
		return fmt.Errorf("link not found")
	}
	link.LastUpdated = time.Now()
	d.links[key] = link
	return nil
}

func (d *dbService) DeleteLink(link Link) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	key := linkStorageKey(link)
	if _, exists := d.links[key]; !exists {
		return fmt.Errorf("link not found")
	}
	delete(d.links, key)
	return nil
}

func (d *dbService) GetLink(linkID string, sourceNodeID string, destNodeID string) (*Link, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	var latest *Link
	for _, link := range d.links {
		if link.Deleted || link.LinkID != linkID {
			continue
		}
		if sourceNodeID != "" && destNodeID != "" {
			direct := link.SourceNodeID == sourceNodeID && link.DestNodeID == destNodeID
			reverse := link.SourceNodeID == destNodeID && link.DestNodeID == sourceNodeID
			if !direct && !reverse {
				continue
			}
		}
		l := link
		if latest == nil || l.LastUpdated.After(latest.LastUpdated) {
			latest = &l
		}
	}
	if latest == nil {
		return nil, fmt.Errorf("link not found")
	}
	return latest, nil
}

func (d *dbService) FindLinkBetweenNodes(sourceNodeID string, destNodeID string) (*Link, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	var latest *Link
	for _, link := range d.links {
		if link.Deleted {
			continue
		}
		direct := link.SourceNodeID == sourceNodeID && link.DestNodeID == destNodeID
		reverse := link.SourceNodeID == destNodeID && link.DestNodeID == sourceNodeID
		if !direct && !reverse {
			continue
		}
		l := link
		if latest == nil || l.LastUpdated.After(latest.LastUpdated) {
			latest = &l
		}
	}
	return latest, nil
}

func (d *dbService) GetLinkForSourceAndEndpoint(sourceNodeID string, destEndpoint string) (*Link, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	var latest *Link
	for _, link := range d.links {
		if link.Deleted {
			continue
		}
		if link.SourceNodeID != sourceNodeID || link.DestEndpoint != destEndpoint {
			continue
		}
		l := link
		if latest == nil || l.LastUpdated.After(latest.LastUpdated) {
			latest = &l
		}
	}
	return latest, nil
}

func (d *dbService) GetLinksForNode(nodeID string) []Link {
	d.mu.RLock()
	defer d.mu.RUnlock()
	links := make([]Link, 0)
	for _, link := range d.links {
		if link.SourceNodeID == nodeID || link.DestNodeID == nodeID {
			links = append(links, link)
		}
	}
	return links
}

func (d *dbService) GetRoutesByLinkID(linkID string) []Route {
	d.mu.RLock()
	defer d.mu.RUnlock()
	routes := make([]Route, 0)
	for _, route := range d.routes {
		if route.LinkID == linkID {
			routes = append(routes, route)
		}
	}
	return routes
}

func (d *dbService) DeleteRoute(routeID uint64) error {
	// Remove route from the map
	d.mu.Lock()
	defer d.mu.Unlock()
	if _, exists := d.routes[routeID]; !exists {
		return fmt.Errorf("route %v not found", routeID)
	}
	delete(d.routes, routeID)
	return nil
}

func (d *dbService) MarkRouteAsDeleted(routeID uint64) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %v not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Deleted = true
	d.routes[routeID] = route
	return nil
}

// MarkRouteAsApplied implements DataAccess.
func (d *dbService) MarkRouteAsApplied(routeID uint64) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %v not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Status = RouteStatusApplied
	d.routes[routeID] = route
	return nil
}

// MarkRouteAsFailed implements DataAccess.
func (d *dbService) MarkRouteAsFailed(routeID uint64, msg string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %v not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.Status = RouteStatusFailed
	route.StatusMsg = msg
	d.routes[routeID] = route
	return nil
}

func (d *dbService) RepointRoute(routeID uint64, linkID string, status RouteStatus, msg string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	route, exists := d.routes[routeID]
	if !exists {
		return fmt.Errorf("route %v not found", routeID)
	}
	route.LastUpdated = time.Now()
	route.LinkID = linkID
	route.Status = status
	route.StatusMsg = msg
	d.routes[routeID] = route
	return nil
}

func (d *dbService) GetRouteByID(routeID uint64) *Route {
	d.mu.RLock()
	defer d.mu.RUnlock()
	if _, exists := d.routes[routeID]; !exists {
		return nil
	}
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
		if route.DestNodeID == nodeID && route.SourceNodeID != AllNodesID {
			routes = append(routes, route)
		}
	}

	// Return empty slice instead of error when no routes found
	return routes
}

func (d *dbService) GetRoutesForDestinationNodeIDAndName(nodeID string, component0 string, component1 string,
	component2 string, componentID *wrapperspb.UInt64Value) []Route {

	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		if route.DestNodeID == nodeID &&
			route.Component0 == component0 &&
			route.Component1 == component1 &&
			route.Component2 == component2 {
			if (componentID == nil && route.ComponentID == nil) ||
				(componentID != nil && route.ComponentID != nil && componentID.Value == route.ComponentID.Value) {
				routes = append(routes, route)
			}
		}
	}
	return routes
}

func (d *dbService) GetRouteForSrcAndDestinationAndName(srcNodeID string, component0 string, component1 string,
	component2 string, componentID *wrapperspb.UInt64Value, destNodeID string, linkID string) (Route, error) {

	d.mu.RLock()
	defer d.mu.RUnlock()
	for _, route := range d.routes {
		if route.SourceNodeID == srcNodeID &&
			(destNodeID == "" || route.DestNodeID == destNodeID) &&
			(linkID == "" || route.LinkID == linkID) &&
			route.Component0 == component0 &&
			route.Component1 == component1 &&
			route.Component2 == component2 {
			if (componentID == nil && route.ComponentID == nil) ||
				(componentID != nil && route.ComponentID != nil && componentID.Value == route.ComponentID.Value) {
				return route, nil
			}
		}
	}
	return Route{}, fmt.Errorf("route not found")
}

func (d *dbService) FilterRoutesBySourceAndDestination(sourceNodeID string, destNodeID string) []Route {
	d.mu.RLock()
	defer d.mu.RUnlock()

	var routes []Route
	for _, route := range d.routes {
		sourceMatch := sourceNodeID == "" || route.SourceNodeID == sourceNodeID
		destMatch := destNodeID == "" || route.DestNodeID == destNodeID
		if sourceMatch && destMatch {
			routes = append(routes, route)
		}
	}

	return routes
}

func (d *dbService) GetDestinationNodeIDForName(component0 string, component1 string, component2 string,
	componentID *wrapperspb.UInt64Value) string {
	d.mu.RLock()
	defer d.mu.RUnlock()

	var matchingRoutes []Route
	for _, route := range d.routes {
		if route.SourceNodeID == AllNodesID &&
			route.Component0 == component0 &&
			route.Component1 == component1 &&
			route.Component2 == component2 {
			if (componentID == nil && route.ComponentID == nil) ||
				(componentID != nil && route.ComponentID != nil && componentID.Value == route.ComponentID.Value) {
				matchingRoutes = append(matchingRoutes, route)
			}
		}
	}

	if len(matchingRoutes) == 0 {
		return ""
	}

	// Sort by LastUpdated descending (most recent first)
	// Find the most recent route
	mostRecent := matchingRoutes[0]
	for _, route := range matchingRoutes[1:] {
		if route.LastUpdated.After(mostRecent.LastUpdated) {
			mostRecent = route
		}
	}

	return mostRecent.DestNodeID
}
