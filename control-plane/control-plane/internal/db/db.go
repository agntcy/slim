package db

import (
	"google.golang.org/protobuf/types/known/wrapperspb"
)

type DataAccess interface {
	ListNodes() ([]Node, error)
	GetNode(id string) (*Node, error)
	SaveNode(node Node) (string, error)
	DeleteNode(id string) error

	ListConnectionsByNodeID(nodeID string) ([]Connection, error)
	SaveConnection(connection Connection) (string, error)
	GetConnection(connectionID string) (Connection, error)
	DeleteConnection(connectionID string) error

	ListSubscriptionsByNodeID(nodeID string) ([]Subscription, error)
	SaveSubscription(subscription Subscription) (string, error)
	GetSubscription(subscriptionID string) (Subscription, error)
	DeleteSubscription(subscriptionID string) error

	SaveChannel(channelID string, moderators []string) error
	DeleteChannel(channelID string) error
	GetChannel(channelID string) (Channel, error)
	UpdateChannel(channel Channel) error
	ListChannels() ([]Channel, error)
}

type Node struct {
	ID   string
	Host string
	Port uint32
}

type Connection struct {
	ID         string
	NodeID     string
	ConfigData string
}

type Subscription struct {
	ID           string
	NodeID       string
	ConnectionID string

	Organization string
	Namespace    string
	AgentType    string
	AgentID      *wrapperspb.UInt64Value
}

type Channel struct {
	ID           string
	Moderators   []string
	Participants []string
}
