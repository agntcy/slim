package db

import (
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/objectbox/objectbox-go/objectbox"
)

// ObjectBox entity structs
type OBNode struct {
	Id          uint64 `objectbox:"id"`
	NodeID      string `objectbox:"unique"`
	GroupName   string
	ConnDetails string // JSON encoded ConnectionDetails
	LastUpdated int64  // Unix timestamp
}

type OBRoute struct {
	Id             uint64 `objectbox:"id"`
	RouteID        string `objectbox:"unique"`
	SourceNodeID   string `objectbox:"index"`
	DestNodeID     string
	DestEndpoint   string
	ConnConfigData string
	Component0     string
	Component1     string
	Component2     string
	ComponentID    uint64
	ComponentIDSet bool
	Deleted        bool
	LastUpdated    int64 // Unix timestamp
}

type OBChannel struct {
	Id           uint64 `objectbox:"id"`
	ChannelID    string `objectbox:"unique"`
	Moderators   string // JSON encoded []string
	Participants string // JSON encoded []string
}

type objectboxService struct {
	ob         *objectbox.ObjectBox
	nodeBox    *objectbox.Box
	routeBox   *objectbox.Box
	channelBox *objectbox.Box
	mu         sync.RWMutex
}

type objectboxService struct {
	ob         *objectbox.ObjectBox
	nodeBox    *objectbox.Box
	routeBox   *objectbox.Box
	channelBox *objectbox.Box
}

func NewObjectBoxDBService(dbPath string) (DataAccess, error) {
	builder := objectbox.NewBuilder().Model(getObjectBoxModel())
	if dbPath != "" {
		builder = builder.Directory(dbPath)
	}

	ob, err := builder.Build()
	if err != nil {
		return nil, fmt.Errorf("failed to create ObjectBox: %w", err)
	}

	return &objectboxService{
		ob:         ob,
		nodeBox:    objectbox.BoxForType(ob, OBNode{}),
		routeBox:   objectbox.BoxForType(ob, OBRoute{}),
		channelBox: objectbox.BoxForType(ob, OBChannel{}),
	}, nil
}

func (o *objectboxService) Close() error {
	if o.ob != nil {
		return o.ob.Close()
	}
	return nil
}

// Node operations
func (o *objectboxService) ListNodes() []Node {
	obNodes, err := o.nodeBox.GetAll()
	if err != nil {
		return []Node{}
	}

	nodes := make([]Node, 0, len(obNodes))
	for _, obj := range obNodes {
		if obNode, ok := obj.(*OBNode); ok {
			node := o.convertFromOBNode(obNode)
			nodes = append(nodes, node)
		}
	}

	return nodes
}

func (o *objectboxService) GetNode(id string) (*Node, error) {
	query := o.nodeBox.Query(OBNode_.NodeID.Equals(id, true))
	objects, err := query.Find()
	query.Close()

	if err != nil {
		return nil, fmt.Errorf("failed to query node: %w", err)
	}

	if len(objects) == 0 {
		return nil, fmt.Errorf("node with ID %s not found", id)
	}

	obNode := objects[0].(*OBNode)
	node := o.convertFromOBNode(obNode)
	return &node, nil
}

func (o *objectboxService) SaveNode(node Node) (string, error) {
	var resultID string
	var resultErr error

	err := o.ob.RunInWriteTx(func() error {
		if node.ID == "" {
			node.ID = uuid.New().String()
		}

		node.LastUpdated = time.Now()

		obNode := o.convertToOBNode(node)

		// Check if node exists and update the ObjectBox ID
		existing, _ := o.getOBNodeByNodeID(node.ID)
		if existing != nil {
			obNode.Id = existing.Id
		}

		_, err := o.nodeBox.Put(obNode)
		if err != nil {
			resultErr = fmt.Errorf("failed to save node: %w", err)
			return resultErr
		}

		resultID = node.ID
		return nil
	})

	if err != nil {
		return "", err
	}

	return resultID, resultErr
}

func (o *objectboxService) DeleteNode(id string) error {
	return o.ob.RunInWriteTx(func() error {
		obNode, err := o.getOBNodeByNodeID(id)
		if err != nil {
			return fmt.Errorf("node with ID %s not found", id)
		}

		err = o.nodeBox.Remove(obNode.Id)
		if err != nil {
			return fmt.Errorf("failed to delete node: %w", err)
		}

		return nil
	})
}

// Route operations
func (o *objectboxService) AddRoute(route Route) string {
	routeID := route.GetID()

	o.ob.RunInWriteTx(func() error {
		route.LastUpdated = time.Now()

		obRoute := o.convertToOBRoute(route, routeID)

		// Check if route exists and update the ObjectBox ID
		existing, _ := o.getOBRouteByRouteID(routeID)
		if existing != nil {
			obRoute.Id = existing.Id
		}

		o.routeBox.Put(obRoute)
		return nil
	})

	return routeID
}

func (o *objectboxService) GetRoutesForNodeID(nodeID string) []Route {
	query := o.routeBox.Query(OBRoute_.SourceNodeID.Equals(nodeID, true))
	objects, err := query.Find()
	query.Close()

	if err != nil {
		return []Route{}
	}

	routes := make([]Route, 0, len(objects))
	for _, obj := range objects {
		if obRoute, ok := obj.(*OBRoute); ok {
			route := o.convertFromOBRoute(obRoute)
			routes = append(routes, route)
		}
	}

	return routes
}

func (o *objectboxService) GetRouteByID(routeID string) *Route {
	obRoute, err := o.getOBRouteByRouteID(routeID)
	if err != nil {
		return &Route{} // Return empty route to match interface behavior
	}

	route := o.convertFromOBRoute(obRoute)
	return &route
}

func (o *objectboxService) DeleteRoute(routeID string) error {
	return o.ob.RunInWriteTx(func() error {
		obRoute, err := o.getOBRouteByRouteID(routeID)
		if err != nil {
			return fmt.Errorf("route %s not found", routeID)
		}

		err = o.routeBox.Remove(obRoute.Id)
		if err != nil {
			return fmt.Errorf("failed to delete route: %w", err)
		}

		return nil
	})
}

func (o *objectboxService) MarkRouteAsDeleted(routeID string) error {
	return o.ob.RunInWriteTx(func() error {
		obRoute, err := o.getOBRouteByRouteID(routeID)
		if err != nil {
			return fmt.Errorf("route %s not found", routeID)
		}

		obRoute.Deleted = true
		obRoute.LastUpdated = time.Now().Unix()

		_, err = o.routeBox.Put(obRoute)
		if err != nil {
			return fmt.Errorf("failed to update route: %w", err)
		}

		return nil
	})
}

// Channel operations
func (o *objectboxService) SaveChannel(channelID string, moderators []string) error {
	return o.ob.RunInWriteTx(func() error {
		// Check if channel exists
		existing, _ := o.getOBChannelByChannelID(channelID)
		if existing != nil {
			return fmt.Errorf("channel with ID %s already exists", channelID)
		}

		obChannel := &OBChannel{
			ChannelID:    channelID,
			Moderators:   o.encodeStringSlice(moderators),
			Participants: o.encodeStringSlice([]string{}),
		}

		_, err := o.channelBox.Put(obChannel)
		if err != nil {
			return fmt.Errorf("failed to save channel: %w", err)
		}

		return nil
	})
}

func (o *objectboxService) DeleteChannel(channelID string) error {
	return o.ob.RunInWriteTx(func() error {
		obChannel, err := o.getOBChannelByChannelID(channelID)
		if err != nil {
			return fmt.Errorf("channel with ID %s not found", channelID)
		}

		err = o.channelBox.Remove(obChannel.Id)
		if err != nil {
			return fmt.Errorf("failed to delete channel: %w", err)
		}

		return nil
	})
}

func (o *objectboxService) GetChannel(channelID string) (Channel, error) {
	obChannel, err := o.getOBChannelByChannelID(channelID)
	if err != nil {
		return Channel{}, fmt.Errorf("channel with ID %s not found", channelID)
	}

	channel := o.convertFromOBChannel(obChannel)
	return channel, nil
}

func (o *objectboxService) UpdateChannel(channel Channel) error {
	return o.ob.RunInWriteTx(func() error {
		if channel.ID == "" {
			return fmt.Errorf("channel ID cannot be empty")
		}

		obChannel, err := o.getOBChannelByChannelID(channel.ID)
		if err != nil {
			return fmt.Errorf("channel with ID %s not found", channel.ID)
		}

		obChannel.Moderators = o.encodeStringSlice(channel.Moderators)
		obChannel.Participants = o.encodeStringSlice(channel.Participants)

		_, err = o.channelBox.Put(obChannel)
		if err != nil {
			return fmt.Errorf("failed to update channel: %w", err)
		}

		return nil
	})
}

func (o *objectboxService) ListChannels() ([]Channel, error) {
	obChannels, err := o.channelBox.GetAll()
	if err != nil {
		return nil, fmt.Errorf("failed to list channels: %w", err)
	}

	channels := make([]Channel, 0, len(obChannels))
	for _, obj := range obChannels {
		if obChannel, ok := obj.(*OBChannel); ok {
			channel := o.convertFromOBChannel(obChannel)
			channels = append(channels, channel)
		}
	}

	return channels, nil
}
