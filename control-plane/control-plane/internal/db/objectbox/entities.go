package objectbox

// ObjectBox entity structs with ObjectBox annotations
type NodeEntity struct {
	Id          uint64 `objectbox:"id"`
	NodeID      string `objectbox:"unique"`
	GroupName   string
	ConnDetails string // JSON serialized ConnectionDetails
	LastUpdated int64  // Unix timestamp
}

type RouteEntity struct {
	Id             uint64 `objectbox:"id"`
	RouteID        string `objectbox:"unique"`
	SourceNodeID   string `objectbox:"index"`
	DestNodeID     string `objectbox:"index"`
	DestEndpoint   string `objectbox:"index"`
	ConnConfigData string
	Component0     string
	Component1     string
	Component2     string
	ComponentID    uint64 // Store as uint64, handle nil separately
	HasComponentID bool   // Flag to indicate if ComponentID is set
	Status         int    // RouteStatus as int
	StatusMsg      string
	Deleted        bool
	LastUpdated    int64 // Unix timestamp
}

type ChannelEntity struct {
	Id           uint64 `objectbox:"id"`
	ChannelID    string `objectbox:"unique"`
	Moderators   string // JSON serialized []string
	Participants string // JSON serialized []string
}
