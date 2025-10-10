package nbapiservice

import (
	"context"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type NodeService struct {
	dbService  db.DataAccess
	cmdHandler nodecontrol.NodeCommandHandler
}

func NewNodeService(dbService db.DataAccess, cmdHandler nodecontrol.NodeCommandHandler) NodeManager {
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
		connDetails = append(connDetails, &controllerapi.ConnectionDetails{
			Endpoint:         conn.Endpoint,
			MtlsRequired:     conn.MTLSRequired,
			ExternalEndpoint: conn.ExternalEndpoint,
			GroupName:        conn.GroupName,
		})
	}
	return connDetails
}
