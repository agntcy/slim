package nbapiservice

import (
	"context"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

type nodeService struct {
	dbService db.DataAccess
}

func NewNodeService(dbService db.DataAccess) *nodeService {
	return &nodeService{
		dbService: dbService,
	}
}

func (s *nodeService) ListNodes(context.Context, *controlplaneApi.NodeListRequest) (*controlplaneApi.NodeListResponse, error) {
	storedNodes, err := s.dbService.ListNodes()
	if err != nil {
		return nil, err
	}
	nodeEntries := make([]*controlplaneApi.NodeEntry, 0, len(storedNodes))
	for _, node := range storedNodes {
		nodeEntry := &controlplaneApi.NodeEntry{
			Id:   node.ID,
			Host: node.Host,
			Port: uint32(node.Port),
		}
		nodeEntries = append(nodeEntries, nodeEntry)
	}
	nodeListresponse := &controlplaneApi.NodeListResponse{
		Entries: nodeEntries,
	}
	return nodeListresponse, nil
}

func (s *nodeService) GetNodeByID(nodeID string) (*controlplaneApi.NodeEntry, error) {
	storedNode, err := s.dbService.GetNodeByID(nodeID)
	if err != nil {
		return nil, err
	}
	nodeEntry := &controlplaneApi.NodeEntry{
		Id:   storedNode.ID,
		Host: storedNode.Host,
		Port: uint32(storedNode.Port),
	}
	return nodeEntry, nil
}

func (s *nodeService) SaveConnection(nodeEntry *controlplaneApi.NodeEntry, connection *controllerapi.Connection) (string, error) {
	//connectionID := uuid.New().String()
	return "a81bc81b-dead-4e5d-abff-90865d1e13b1", nil
}

func (s *nodeService) GetConnectionDetails(nodeID string, connectionID string) (string, string, error) {
	return "http://127.0.0.1:46357", `{"endpoint": "http://127.0.0.1:46357"}`, nil
}

func (s *nodeService) SaveSubscription(nodeID string, subscription *controllerapi.Subscription) (string, error) {
	return "6a39545c-00ef-460d-8223-be4816126ef6", nil
}

func (s *nodeService) GetSubscription(nodeID string, subscriptionId string) (*controllerapi.Subscription, error) {
	return &controllerapi.Subscription{
		Organization: "org",
		Namespace:    "default",
		AgentType:    "a",
		ConnectionId: "http://127.0.0.1:46357",
		AgentId:      wrapperspb.UInt64(0),
	}, nil
}
