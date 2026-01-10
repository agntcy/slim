package nbapiservice

import (
	"context"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"google.golang.org/protobuf/types/known/structpb"
)

type NodeServiceDataAccess interface {
	ListNodes() []db.Node
	GetNode(id string) (*db.Node, error)
}

type NodeServiceNodeCommandHandler interface {
	GetConnectionStatus(ctx context.Context, nodeID string) (nodecontrol.NodeStatus, error)
}

type NodeService struct {
	dbService  NodeServiceDataAccess
	cmdHandler NodeServiceNodeCommandHandler
}

func NewNodeService(dbService NodeServiceDataAccess, cmdHandler NodeServiceNodeCommandHandler) NodeManager {
	return &NodeService{
		dbService:  dbService,
		cmdHandler: cmdHandler,
	}
}

func (s *NodeService) ListNodes(
	ctx context.Context, _ *controlplaneApi.NodeListRequest,
) (*controlplaneApi.NodeListResponse, error) {
	storedNodes := s.dbService.ListNodes()
	nodeEntries := make([]*controlplaneApi.NodeEntry, 0, len(storedNodes))
	for _, node := range storedNodes {
		nodeEntry := &controlplaneApi.NodeEntry{
			Id: node.ID,
		}
		// add connection details if available
		nodeEntry.Connections = getNodeConnDetails(node)
		nodeStatus, _ := s.cmdHandler.GetConnectionStatus(ctx, node.ID)
		switch nodeStatus {
		case nodecontrol.NodeStatusConnected:
			nodeEntry.Status = controlplaneApi.NodeStatus_CONNECTED
		case nodecontrol.NodeStatusNotConnected:
			nodeEntry.Status = controlplaneApi.NodeStatus_NOT_CONNECTED
		}
		nodeEntries = append(nodeEntries, nodeEntry)
	}
	nodeListresponse := &controlplaneApi.NodeListResponse{
		Entries: nodeEntries,
	}
	return nodeListresponse, nil
}

func (s *NodeService) GetNodeByID(nodeID string) (*controlplaneApi.NodeEntry, error) {
	storedNode, err := s.dbService.GetNode(nodeID)
	if err != nil {
		return nil, err
	}
	nodeEntry := &controlplaneApi.NodeEntry{
		Id: storedNode.ID,
	}
	// add connection details if available
	nodeEntry.Connections = getNodeConnDetails(*storedNode)
	return nodeEntry, nil
}

func getNodeConnDetails(node db.Node) []*controllerapi.ConnectionDetails {
	connDetails := make([]*controllerapi.ConnectionDetails, 0, len(node.ConnDetails))
	for _, conn := range node.ConnDetails {
		cd := &controllerapi.ConnectionDetails{
			Endpoint:     conn.Endpoint,
			MtlsRequired: conn.MTLSRequired,
		}

		// Add ExternalEndpoint to metadata if present
		if conn.ExternalEndpoint != nil {
			if cd.Metadata == nil {
				cd.Metadata = &structpb.Struct{
					Fields: make(map[string]*structpb.Value),
				}
			}
			cd.Metadata.Fields["external_endpoint"] = structpb.NewStringValue(*conn.ExternalEndpoint)
		}

		connDetails = append(connDetails, cd)
	}
	return connDetails
}
