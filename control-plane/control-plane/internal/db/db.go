package db

import (
	"google.golang.org/protobuf/types/known/wrapperspb"
)

const AllNodesID = "*"

type DataAccess interface {
	ListNodes() []Node
	GetNode(id string) (*Node, error)
	SaveNode(node Node) (string, bool, error)
	DeleteNode(id string) error

	AddRoute(route Route) (Route, error)
	GetRoutesForNodeID(nodeID string) []Route
	GetRoutesForDestinationNodeID(nodeID string) []Route
	GetRoutesForDestinationNodeIDAndName(nodeID string, Component0 string, Component1 string,
		Component2 string, ComponentID *wrapperspb.UInt64Value) []Route
	// FilterRoutesBySourceAndDestination returns all routes matching the given sourceNodeID and destNodeID if any.
	// If any of the parameters is an empty string, it is treated as a wildcard.
	FilterRoutesBySourceAndDestination(sourceNodeID string, destNodeID string) []Route

	GetRouteForSrcAndDestinationAndName(srcNodeID string, Component0 string, Component1 string,
		Component2 string, ComponentID *wrapperspb.UInt64Value, destNodeID string, destEndpoint string) (Route, error)

	// GetDestinationNodeIDForName queries for routes with srcNodeID = ALL and component names,
	// orders routes by last updated time (first being the latest) and returns dest nodeID of first route.
	GetDestinationNodeIDForName(Component0 string, Component1 string,
		Component2 string, ComponentID *wrapperspb.UInt64Value) string

	GetRouteByID(routeID uint64) *Route
	DeleteRoute(routeID uint64) error
	MarkRouteAsDeleted(routeID uint64) error
	MarkRouteAsApplied(routeID uint64) error
	MarkRouteAsFailed(routeID uint64, msg string) error

	SaveChannel(channelID string, moderators []string) error
	DeleteChannel(channelID string) error
	GetChannel(channelID string) (Channel, error)
	UpdateChannel(channel Channel) error
	ListChannels() ([]Channel, error)
}
