package db

import (
	"fmt"
	"time"

	"google.golang.org/protobuf/types/known/wrapperspb"
)

type DataAccess interface {
	ListNodes() []Node
	GetNode(id string) (*Node, error)
	SaveNode(node Node) (string, error)
	DeleteNode(id string) error

	AddRoute(route Route) string
	GetRoutesForNodeID(nodeID string) []Route
	GetRouteByID(routeID string) *Route
	DeleteRoute(routeID string) error
	MarkRouteAsDeleted(routeID string) error

	SaveChannel(channelID string, moderators []string) error
	DeleteChannel(channelID string) error
	GetChannel(channelID string) (Channel, error)
	UpdateChannel(channel Channel) error
	ListChannels() ([]Channel, error)
}

type Node struct {
	ID          string
	ConnDetails []ConnectionDetails
	LastUpdated time.Time
}

type ConnectionDetails struct {
	Endpoint         string
	ExternalEndpoint *string
	GroupName        *string
	MTLSRequired     bool
}

type Route struct {
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

	Deleted     bool
	LastUpdated time.Time
}

func (r Route) GetID() string {
	destID := r.DestNodeID

	if destID == "" {
		destID = r.DestEndpoint
	}

	return fmt.Sprintf("%s:%s/%s/%s/%v->%s", r.SourceNodeID,
		r.Component0, r.Component1, r.Component2, r.ComponentID, destID)
}

type Channel struct {
	ID           string
	Moderators   []string
	Participants []string
}
