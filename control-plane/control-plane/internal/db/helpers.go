package db

import (
	"fmt"
	"time"

	"github.com/objectbox/objectbox-go/objectbox"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

// Helper methods for conversion and queries
func (o *objectboxService) getOBNodeByNodeID(nodeID string) (*OBNode, error) {
	query := o.nodeBox.Query(OBNode_.NodeID.Equals(nodeID, true))
	objects, err := query.Find()
	query.Close()

	if err != nil || len(objects) == 0 {
		return nil, fmt.Errorf("node not found")
	}

	return objects[0].(*OBNode), nil
}

func (o *objectboxService) getOBRouteByRouteID(routeID string) (*OBRoute, error) {
	query := o.routeBox.Query(OBRoute_.RouteID.Equals(routeID, true))
	objects, err := query.Find()
	query.Close()

	if err != nil || len(objects) == 0 {
		return nil, fmt.Errorf("route not found")
	}

	return objects[0].(*OBRoute), nil
}

func (o *objectboxService) getOBChannelByChannelID(channelID string) (*OBChannel, error) {
	query := o.channelBox.Query(OBChannel_.ChannelID.Equals(channelID, true))
	objects, err := query.Find()
	query.Close()

	if err != nil || len(objects) == 0 {
		return nil, fmt.Errorf("channel not found")
	}

	return objects[0].(*OBChannel), nil
}

func (o *objectboxService) convertToOBNode(node Node) *OBNode {
	groupName := ""
	if node.GroupName != nil {
		groupName = *node.GroupName
	}

	return &OBNode{
		NodeID:      node.ID,
		GroupName:   groupName,
		ConnDetails: o.encodeConnectionDetails(node.ConnDetails),
		LastUpdated: node.LastUpdated.Unix(),
	}
}

func (o *objectboxService) convertFromOBNode(obNode *OBNode) Node {
	var groupName *string
	if obNode.GroupName != "" {
		groupName = &obNode.GroupName
	}

	return Node{
		ID:          obNode.NodeID,
		GroupName:   groupName,
		ConnDetails: o.decodeConnectionDetails(obNode.ConnDetails),
		LastUpdated: time.Unix(obNode.LastUpdated, 0),
	}
}

func (o *objectboxService) convertToOBRoute(route Route, routeID string) *OBRoute {
	var componentID uint64
	componentIDSet := false
	if route.ComponentID != nil {
		componentID = route.ComponentID.Value
		componentIDSet = true
	}

	return &OBRoute{
		RouteID:        routeID,
		SourceNodeID:   route.SourceNodeID,
		DestNodeID:     route.DestNodeID,
		DestEndpoint:   route.DestEndpoint,
		ConnConfigData: route.ConnConfigData,
		Component0:     route.Component0,
		Component1:     route.Component1,
		Component2:     route.Component2,
		ComponentID:    componentID,
		ComponentIDSet: componentIDSet,
		Deleted:        route.Deleted,
		LastUpdated:    route.LastUpdated.Unix(),
	}
}

func (o *objectboxService) convertFromOBRoute(obRoute *OBRoute) Route {
	var componentID *wrapperspb.UInt64Value
	if obRoute.ComponentIDSet {
		componentID = &wrapperspb.UInt64Value{Value: obRoute.ComponentID}
	}

	return Route{
		SourceNodeID:   obRoute.SourceNodeID,
		DestNodeID:     obRoute.DestNodeID,
		DestEndpoint:   obRoute.DestEndpoint,
		ConnConfigData: obRoute.ConnConfigData,
		Component0:     obRoute.Component0,
		Component1:     obRoute.Component1,
		Component2:     obRoute.Component2,
		ComponentID:    componentID,
		Deleted:        obRoute.Deleted,
		LastUpdated:    time.Unix(obRoute.LastUpdated, 0),
	}
}

func (o *objectboxService) convertFromOBChannel(obChannel *OBChannel) Channel {
	return Channel{
		ID:           obChannel.ChannelID,
		Moderators:   o.decodeStringSlice(obChannel.Moderators),
		Participants: o.decodeStringSlice(obChannel.Participants),
	}
}

// JSON encoding/decoding helpers (implement these based on your needs)
func (o *objectboxService) encodeConnectionDetails(details []ConnectionDetails) string {
	// Implement JSON encoding
	return ""
}

func (o *objectboxService) decodeConnectionDetails(data string) []ConnectionDetails {
	// Implement JSON decoding
	return []ConnectionDetails{}
}

func (o *objectboxService) encodeStringSlice(slice []string) string {
	// Implement JSON encoding
	return ""
}

func (o *objectboxService) decodeStringSlice(data string) []string {
	// Implement JSON decoding
	return []string{}
}

func getObjectBoxModel() *objectbox.Model {
	// You'll need to generate this using ObjectBox model generator
	// Run: go run github.com/objectbox/objectbox-go/cmd/objectbox-gogen
	return nil
}
