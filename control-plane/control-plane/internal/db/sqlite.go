package db

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"time"

	"github.com/glebarez/sqlite"
	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"google.golang.org/protobuf/types/known/wrapperspb"
	"gorm.io/gorm"
)

type SQLiteDBService struct {
	db     *gorm.DB
	dbPath string
}

// GORM model structs
type NodeModel struct {
	ID              string `gorm:"primaryKey"`
	GroupName       *string
	ConnDetailsJSON string `gorm:"column:conn_details"`
	LastUpdated     time.Time
}

func (NodeModel) TableName() string {
	return "nodes"
}

type RouteModel struct {
	ID             uint64 `gorm:"primaryKey"`
	SourceNodeID   string
	DestNodeID     string
	DestEndpoint   string
	ConnConfigData string
	Component0     string
	Component1     string
	Component2     string
	ComponentID    *string
	Status         int
	StatusMsg      string
	Deleted        bool
	LastUpdated    time.Time
}

func (RouteModel) TableName() string {
	return "routes"
}

type ChannelModel struct {
	ID               string `gorm:"primaryKey"`
	ModeratorsJSON   string `gorm:"column:moderators"`
	ParticipantsJSON string `gorm:"column:participants"`
}

func (ChannelModel) TableName() string {
	return "channels"
}

// NewSQLiteDBService creates a new SQLite database service
func NewSQLiteDBService(ctx context.Context, dbPath string) (DataAccess, error) {
	db, err := gorm.Open(sqlite.Open(dbPath), &gorm.Config{})
	if err != nil {
		return nil, fmt.Errorf("failed to connect to database: %w (path: %s)", err, dbPath)
	}

	service := &SQLiteDBService{db: db, dbPath: dbPath}

	// Auto-migrate the schema
	if err := service.migrate(); err != nil {
		return nil, fmt.Errorf("failed to migrate database: %w", err)
	}
	zlog := zerolog.Ctx(ctx)
	zlog.Debug().Msgf("sqlite database initialized: %s", dbPath)
	return service, nil
}

func (s *SQLiteDBService) migrate() error {
	return s.db.AutoMigrate(&NodeModel{}, &RouteModel{}, &ChannelModel{})
}

// Node operations
func (s *SQLiteDBService) ListNodes() []Node {
	var nodeModels []NodeModel
	s.db.Find(&nodeModels)

	nodes := make([]Node, len(nodeModels))
	for i, nm := range nodeModels {
		nodes[i] = s.nodeModelToNode(nm)
	}
	return nodes
}

func (s *SQLiteDBService) GetNode(id string) (*Node, error) {
	var nodeModel NodeModel
	if err := s.db.First(&nodeModel, "id = ?", id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, fmt.Errorf("node with ID %s not found", id)
		}
		return nil, err
	}

	node := s.nodeModelToNode(nodeModel)
	return &node, nil
}

func (s *SQLiteDBService) SaveNode(node Node) (string, bool, error) {
	connDetailsChanged := false

	if node.ID == "" {
		node.ID = uuid.New().String()
	} else {
		// Check if node exists and compare connection details
		existing, err := s.GetNode(node.ID)
		if err == nil {
			connDetailsChanged = hasConnectionDetailsChanged(existing.ConnDetails, node.ConnDetails)
		}
	}

	node.LastUpdated = time.Now()
	nodeModel := s.nodeToNodeModel(node)

	if err := s.db.Save(&nodeModel).Error; err != nil {
		return "", false, err
	}

	return node.ID, connDetailsChanged, nil
}

func (s *SQLiteDBService) DeleteNode(id string) error {
	result := s.db.Delete(&NodeModel{}, "id = ?", id)
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("node with ID %s not found", id)
	}
	return nil
}

// Route operations
func (s *SQLiteDBService) AddRoute(route Route) (Route, error) {
	route.ID = route.GetUniqueID()
	route.LastUpdated = time.Now()

	routeModel := s.routeToRouteModel(route)
	s.db.Save(&routeModel)

	return route, nil
}

func (s *SQLiteDBService) GetRoutesForNodeID(nodeID string) []Route {
	var routeModels []RouteModel
	s.db.Where("source_node_id = ?", nodeID).Find(&routeModels)

	routes := make([]Route, len(routeModels))
	for i, rm := range routeModels {
		routes[i] = s.routeModelToRoute(rm)
	}
	return routes
}

func (s *SQLiteDBService) GetRoutesForDestinationNodeID(nodeID string) []Route {
	var routeModels []RouteModel
	s.db.Where("dest_node_id = ? AND source_node_id != ?", nodeID, AllNodesID).Find(&routeModels)

	routes := make([]Route, len(routeModels))
	for i, rm := range routeModels {
		routes[i] = s.routeModelToRoute(rm)
	}
	return routes
}

func (s *SQLiteDBService) GetRoutesForDestinationNodeIDAndName(nodeID string, component0 string, component1 string,
	component2 string, componentID *wrapperspb.UInt64Value) []Route {

	query := s.db.Where("dest_node_id = ? AND component0 = ? AND component1 = ? AND component2 = ?",
		nodeID, component0, component1, component2)

	if componentID == nil {
		query = query.Where("component_id IS NULL")
	} else {
		componentIDStr := strconv.FormatUint(componentID.Value, 10)
		query = query.Where("component_id = ?", componentIDStr)
	}

	var routeModels []RouteModel
	query.Find(&routeModels)

	routes := make([]Route, len(routeModels))
	for i, rm := range routeModels {
		routes[i] = s.routeModelToRoute(rm)
	}
	return routes
}

func (s *SQLiteDBService) GetRouteForSrcAndDestinationAndName(srcNodeID string, component0 string, component1 string,
	component2 string, componentID *wrapperspb.UInt64Value, destNodeID string, destEndpoint string) (Route, error) {

	query := s.db.Where("source_node_id = ? AND component0 = ? AND component1 = ? AND component2 = ?",
		srcNodeID, component0, component1, component2)

	if destNodeID != "" {
		query = query.Where("dest_node_id = ?", destNodeID)
	}
	if destEndpoint != "" {
		query = query.Where("dest_endpoint = ?", destEndpoint)
	}

	if componentID == nil {
		query = query.Where("component_id IS NULL")
	} else {
		componentIDStr := strconv.FormatUint(componentID.Value, 10)
		query = query.Where("component_id = ?", componentIDStr)
	}

	var routeModel RouteModel
	if err := query.First(&routeModel).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return Route{}, fmt.Errorf("route not found")
		}
		return Route{}, err
	}

	return s.routeModelToRoute(routeModel), nil
}

func (s *SQLiteDBService) FilterRoutesBySourceAndDestination(sourceNodeID string, destNodeID string) []Route {
	query := s.db.Model(&RouteModel{})

	if sourceNodeID != "" {
		query = query.Where("source_node_id = ?", sourceNodeID)
	}
	if destNodeID != "" {
		query = query.Where("dest_node_id = ?", destNodeID)
	}

	var routeModels []RouteModel
	query.Find(&routeModels)

	routes := make([]Route, len(routeModels))
	for i, rm := range routeModels {
		routes[i] = s.routeModelToRoute(rm)
	}
	return routes
}

func (s *SQLiteDBService) GetDestinationNodeIDForName(component0 string, component1 string, component2 string, componentID *wrapperspb.UInt64Value) string {
	query := s.db.Model(&RouteModel{}).
		Where("source_node_id = ? AND component0 = ? AND component1 = ? AND component2 = ?",
			AllNodesID, component0, component1, component2)

	if componentID == nil {
		query = query.Where("component_id IS NULL")
	} else {
		componentIDStr := strconv.FormatUint(componentID.Value, 10)
		query = query.Where("component_id = ?", componentIDStr)
	}

	// Order by last_updated descending to get the most recent route first
	query = query.Order("last_updated DESC")

	var routeModel RouteModel
	if err := query.First(&routeModel).Error; err != nil {
		return ""
	}

	return routeModel.DestNodeID
}

func (s *SQLiteDBService) GetRouteByID(routeID uint64) *Route {
	var routeModel RouteModel
	if err := s.db.First(&routeModel, "id = ?", routeID).Error; err != nil {
		return nil
	}

	route := s.routeModelToRoute(routeModel)
	return &route
}

func (s *SQLiteDBService) DeleteRoute(routeID uint64) error {
	result := s.db.Delete(&RouteModel{}, "id = ?", routeID)
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("route %v not found", routeID)
	}
	return nil
}

func (s *SQLiteDBService) MarkRouteAsDeleted(routeID uint64) error {
	result := s.db.Model(&RouteModel{}).Where("id = ?", routeID).Updates(map[string]interface{}{
		"deleted":      true,
		"last_updated": time.Now(),
	})
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("route %v not found", routeID)
	}
	return nil
}

func (s *SQLiteDBService) MarkRouteAsApplied(routeID uint64) error {
	result := s.db.Model(&RouteModel{}).Where("id = ?", routeID).Updates(map[string]interface{}{
		"status":       int(RouteStatusApplied),
		"last_updated": time.Now(),
	})
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("route %v not found", routeID)
	}
	return nil
}

func (s *SQLiteDBService) MarkRouteAsFailed(routeID uint64, msg string) error {
	result := s.db.Model(&RouteModel{}).Where("id = ?", routeID).Updates(map[string]interface{}{
		"status":       int(RouteStatusFailed),
		"status_msg":   msg,
		"last_updated": time.Now(),
	})
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("route %v not found", routeID)
	}
	return nil
}

// Channel operations
func (s *SQLiteDBService) SaveChannel(channelID string, moderators []string) error {
	var existing ChannelModel
	if err := s.db.First(&existing, "id = ?", channelID).Error; err == nil {
		return fmt.Errorf("channel with ID %s already exists", channelID)
	}

	moderatorsJSON, _ := json.Marshal(moderators)
	participantsJSON, _ := json.Marshal([]string{})

	channel := ChannelModel{
		ID:               channelID,
		ModeratorsJSON:   string(moderatorsJSON),
		ParticipantsJSON: string(participantsJSON),
	}

	return s.db.Create(&channel).Error
}

func (s *SQLiteDBService) DeleteChannel(channelID string) error {
	result := s.db.Delete(&ChannelModel{}, "id = ?", channelID)
	if result.Error != nil {
		return result.Error
	}
	if result.RowsAffected == 0 {
		return fmt.Errorf("channel with ID %s not found", channelID)
	}
	return nil
}

func (s *SQLiteDBService) GetChannel(channelID string) (Channel, error) {
	var channelModel ChannelModel
	if err := s.db.First(&channelModel, "id = ?", channelID).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return Channel{}, fmt.Errorf("channel with ID %s not found", channelID)
		}
		return Channel{}, err
	}

	return s.channelModelToChannel(channelModel), nil
}

func (s *SQLiteDBService) UpdateChannel(channel Channel) error {
	if channel.ID == "" {
		return fmt.Errorf("channel ID cannot be empty")
	}

	var existing ChannelModel
	if err := s.db.First(&existing, "id = ?", channel.ID).Error; err != nil {
		return fmt.Errorf("channel with ID %s not found", channel.ID)
	}

	channelModel := s.channelToChannelModel(channel)
	return s.db.Save(&channelModel).Error
}

func (s *SQLiteDBService) ListChannels() ([]Channel, error) {
	var channelModels []ChannelModel
	if err := s.db.Find(&channelModels).Error; err != nil {
		return nil, err
	}

	channels := make([]Channel, len(channelModels))
	for i, cm := range channelModels {
		channels[i] = s.channelModelToChannel(cm)
	}
	return channels, nil
}

// Helper methods for model conversion
func (s *SQLiteDBService) nodeToNodeModel(node Node) NodeModel {
	connDetailsJSON, _ := json.Marshal(node.ConnDetails)
	return NodeModel{
		ID:              node.ID,
		GroupName:       node.GroupName,
		ConnDetailsJSON: string(connDetailsJSON),
		LastUpdated:     node.LastUpdated,
	}
}

func (s *SQLiteDBService) nodeModelToNode(nodeModel NodeModel) Node {
	var connDetails []ConnectionDetails
	_ = json.Unmarshal([]byte(nodeModel.ConnDetailsJSON), &connDetails)

	return Node{
		ID:          nodeModel.ID,
		GroupName:   nodeModel.GroupName,
		ConnDetails: connDetails,
		LastUpdated: nodeModel.LastUpdated,
	}
}

func (s *SQLiteDBService) routeToRouteModel(route Route) RouteModel {
	var componentID *string
	if route.ComponentID != nil {
		componentIDStr := strconv.FormatUint(route.ComponentID.Value, 10)
		componentID = &componentIDStr
	}

	return RouteModel{
		ID:             route.ID,
		SourceNodeID:   route.SourceNodeID,
		DestNodeID:     route.DestNodeID,
		DestEndpoint:   route.DestEndpoint,
		ConnConfigData: route.ConnConfigData,
		Component0:     route.Component0,
		Component1:     route.Component1,
		Component2:     route.Component2,
		ComponentID:    componentID,
		Status:         int(route.Status),
		StatusMsg:      route.StatusMsg,
		Deleted:        route.Deleted,
		LastUpdated:    route.LastUpdated,
	}
}

func (s *SQLiteDBService) routeModelToRoute(routeModel RouteModel) Route {
	var componentID *wrapperspb.UInt64Value
	if routeModel.ComponentID != nil {
		if componentIDValue, err := strconv.ParseUint(*routeModel.ComponentID, 10, 64); err == nil {
			componentID = &wrapperspb.UInt64Value{Value: componentIDValue}
		}
	}

	return Route{
		ID:             routeModel.ID,
		SourceNodeID:   routeModel.SourceNodeID,
		DestNodeID:     routeModel.DestNodeID,
		DestEndpoint:   routeModel.DestEndpoint,
		ConnConfigData: routeModel.ConnConfigData,
		Component0:     routeModel.Component0,
		Component1:     routeModel.Component1,
		Component2:     routeModel.Component2,
		ComponentID:    componentID,
		Status:         RouteStatus(routeModel.Status),
		StatusMsg:      routeModel.StatusMsg,
		Deleted:        routeModel.Deleted,
		LastUpdated:    routeModel.LastUpdated,
	}
}

func (s *SQLiteDBService) channelToChannelModel(channel Channel) ChannelModel {
	moderatorsJSON, _ := json.Marshal(channel.Moderators)
	participantsJSON, _ := json.Marshal(channel.Participants)

	return ChannelModel{
		ID:               channel.ID,
		ModeratorsJSON:   string(moderatorsJSON),
		ParticipantsJSON: string(participantsJSON),
	}
}

func (s *SQLiteDBService) channelModelToChannel(channelModel ChannelModel) Channel {
	var moderators, participants []string
	_ = json.Unmarshal([]byte(channelModel.ModeratorsJSON), &moderators)
	_ = json.Unmarshal([]byte(channelModel.ParticipantsJSON), &participants)

	return Channel{
		ID:           channelModel.ID,
		Moderators:   moderators,
		Participants: participants,
	}
}
