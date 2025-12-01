package sbapiservice

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

// MockSlimServer emulates a data-plane node speaking ControllerService over gRPC.
type MockSlimServer struct {
	Name string

	// client
	conn       *grpc.ClientConn
	client     controllerapi.ControllerServiceClient
	stream     controllerapi.ControllerService_OpenControlChannelClient
	dialTarget string

	// registration params
	Port             int32
	GroupName        *string
	LocalEndpoint    *string
	ExternalEndpoint *string
	MTLSRequired     bool

	// behavior flags (for tests)
	AckConnectionError         bool
	AckSubscriptionSetError    bool
	AckSubscriptionDeleteError bool

	// observed state
	mu                sync.RWMutex
	recvConnections   map[string]*controllerapi.Connection
	recvSubscriptions map[string]*controllerapi.Subscription
}

func NewMockSlimServer(name string, port int32, dialTarget string) (*MockSlimServer, error) {
	return &MockSlimServer{Name: name, dialTarget: dialTarget, Port: port}, nil
}

// Start creates the gRPC client and runs connect in a separate goroutine.
func (m *MockSlimServer) Start(ctx context.Context) error {
	conn, err := grpc.NewClient(
		m.dialTarget,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		return err
	}
	m.conn = conn
	m.client = controllerapi.NewControllerServiceClient(conn)
	return m.connect(ctx)
}

// connect dials stream, registers, then starts a receive loop.
func (m *MockSlimServer) connect(ctx context.Context) error {
	var err error
	m.stream, err = m.client.OpenControlChannel(ctx)
	if err != nil {
		return err
	}

	// First wait for server ACK on new stream
	ackMsg, err := m.stream.Recv()
	if err != nil {
		return fmt.Errorf("waiting initial ACK failed: %w", err)
	}
	if ack := ackMsg.GetAck(); ack == nil {
		return fmt.Errorf("expected initial ACK, got %T", ackMsg.Payload)
	}

	uid, _ := uuid.NewUUID()
	// Then send registration
	reg := &controllerapi.ControlMessage{
		MessageId: uid.String(),
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
			NodeId:    m.Name,
			GroupName: m.GroupName,
			ConnectionDetails: []*controllerapi.ConnectionDetails{{
				Endpoint:         fmt.Sprintf("127.0.0.1:%v", m.Port),
				MtlsRequired:     m.MTLSRequired,
				LocalEndpoint:    m.LocalEndpoint,
				ExternalEndpoint: m.ExternalEndpoint,
			}},
		}},
	}
	if err := m.stream.Send(reg); err != nil {
		return fmt.Errorf("register send failed: %w", err)
	}

	go m.recvLoop()
	return nil
}

func (m *MockSlimServer) recvLoop() {
	for {
		msg, err := m.stream.Recv()
		if err != nil {
			fmt.Printf("recvLoop err: %v", err)
			return
		}
		switch p := msg.Payload.(type) {
		case *controllerapi.ControlMessage_ConfigCommand:
			m.handleConfigCommand(msg.MessageId, p.ConfigCommand)
		case *controllerapi.ControlMessage_Ack:
		default:
			// ignore
		}
	}
}

func (m *MockSlimServer) handleConfigCommand(origMsgID string, cfg *controllerapi.ConfigurationCommand) {
	m.mu.Lock()
	for _, c := range cfg.ConnectionsToCreate {
		if m.recvConnections == nil {
			m.recvConnections = make(map[string]*controllerapi.Connection)
		}
		m.recvConnections[c.ConnectionId] = c
	}
	for _, c := range cfg.SubscriptionsToSet {
		if m.recvSubscriptions == nil {
			m.recvSubscriptions = make(map[string]*controllerapi.Subscription)
		}
		key := fmt.Sprintf("%s/%s/%s/%d-->%s", c.Component_0, c.Component_1, c.Component_2, c.GetId().GetValue(),
			c.ConnectionId)
		m.recvSubscriptions[key] = c
	}
	for _, c := range cfg.SubscriptionsToDelete {
		if m.recvSubscriptions != nil {
			key := fmt.Sprintf("%s/%s/%s/%d-->%s", c.Component_0, c.Component_1, c.Component_2, c.GetId().GetValue(),
				c.ConnectionId)
			delete(m.recvSubscriptions, key)
		}
	}
	m.mu.Unlock()
	uid, _ := uuid.NewUUID()
	ack := &controllerapi.ControlMessage{
		MessageId: uid.String(),
		Payload: &controllerapi.ControlMessage_ConfigCommandAck{
			ConfigCommandAck: &controllerapi.ConfigurationCommandAck{OriginalMessageId: origMsgID}}}
	// inject errors if requested
	for _, c := range cfg.ConnectionsToCreate {
		cack := &controllerapi.ConnectionAck{ConnectionId: c.ConnectionId, Success: true}
		if m.AckConnectionError {
			cack.Success = false
			cack.ErrorMsg = "conn error"
		}
		ack.GetConfigCommandAck().ConnectionsStatus = append(ack.GetConfigCommandAck().ConnectionsStatus, cack)
	}
	for _, s := range cfg.SubscriptionsToSet {
		sack := &controllerapi.SubscriptionAck{
			Subscription: s,
			Success:      true,
		}
		if m.AckSubscriptionSetError {
			sack.Success = false
			sack.ErrorMsg = "sub error"
		}
		ack.GetConfigCommandAck().SubscriptionsStatus = append(ack.GetConfigCommandAck().SubscriptionsStatus, sack)
	}
	for _, s := range cfg.SubscriptionsToDelete {
		sack := &controllerapi.SubscriptionAck{
			Subscription: s,
			Success:      true,
		}
		if m.AckSubscriptionDeleteError {
			sack.Success = false
			sack.ErrorMsg = "sub error"
		}
		ack.GetConfigCommandAck().SubscriptionsStatus = append(ack.GetConfigCommandAck().SubscriptionsStatus, sack)
	}

	err := m.stream.Send(ack)
	if err != nil {
		fmt.Printf("handleConfigCommand: send ACK error: %v\n", err)
	}
}

// updateSubscription sends a single subscription config command (add or delete when delete=true).
func (m *MockSlimServer) updateSubscription(_ context.Context, comp0, comp1,
	comp2 string, id uint64, deleteSub bool) error {
	payload := &controllerapi.ConfigurationCommand{}
	if deleteSub {
		payload.SubscriptionsToDelete = []*controllerapi.Subscription{{
			Component_0:  comp0,
			Component_1:  comp1,
			Component_2:  comp2,
			Id:           wrapperspb.UInt64(id),
			ConnectionId: "n/a",
		}}
	} else {
		payload.SubscriptionsToSet = []*controllerapi.Subscription{{
			Component_0:  comp0,
			Component_1:  comp1,
			Component_2:  comp2,
			Id:           wrapperspb.UInt64(id),
			ConnectionId: "n/a",
		}}
	}
	msg := &controllerapi.ControlMessage{MessageId: fmt.Sprintf("%s-cfg-%d", m.Name, time.Now().UnixNano()),
		Payload: &controllerapi.ControlMessage_ConfigCommand{ConfigCommand: payload}}
	return m.stream.Send(msg)
}

func (m *MockSlimServer) Close() error {
	if m.stream != nil {
		_ = m.stream.CloseSend()
	}
	if m.conn != nil {
		return m.conn.Close()
	}
	return nil
}

func (m *MockSlimServer) GetReceived() (conns map[string]*controllerapi.Connection,
	subs map[string]*controllerapi.Subscription) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.recvConnections, m.recvSubscriptions
}

// mockGroupService implements groupservice.GroupManager for testing
type mockGroupService struct {
	createChannelResponse     *controlplaneApi.CreateChannelResponse
	createChannelError        error
	deleteChannelResponse     *controllerapi.Ack
	deleteChannelError        error
	addParticipantResponse    *controllerapi.Ack
	addParticipantError       error
	deleteParticipantResponse *controllerapi.Ack
	deleteParticipantError    error
	listChannelsResponse      *controllerapi.ListChannelsResponse
	listChannelsError         error
	listParticipantsResponse  *controllerapi.ListParticipantsResponse
	listParticipantsError     error
}

func (m *mockGroupService) CreateChannel(
	_ context.Context,
	_ *controlplaneApi.CreateChannelRequest,
) (*controlplaneApi.CreateChannelResponse, error) {
	if m.createChannelError != nil {
		return nil, m.createChannelError
	}
	return m.createChannelResponse, nil
}

func (m *mockGroupService) DeleteChannel(
	_ context.Context,
	_ *controllerapi.DeleteChannelRequest,
) (*controllerapi.Ack, error) {
	if m.deleteChannelError != nil {
		return nil, m.deleteChannelError
	}
	return m.deleteChannelResponse, nil
}

func (m *mockGroupService) AddParticipant(
	_ context.Context,
	_ *controllerapi.AddParticipantRequest,
) (*controllerapi.Ack, error) {
	if m.addParticipantError != nil {
		return nil, m.addParticipantError
	}
	return m.addParticipantResponse, nil
}

func (m *mockGroupService) DeleteParticipant(
	_ context.Context,
	_ *controllerapi.DeleteParticipantRequest,
) (*controllerapi.Ack, error) {
	if m.deleteParticipantError != nil {
		return nil, m.deleteParticipantError
	}
	return m.deleteParticipantResponse, nil
}

func (m *mockGroupService) ListChannels(
	_ context.Context,
	_ *controllerapi.ListChannelsRequest,
) (*controllerapi.ListChannelsResponse, error) {
	if m.listChannelsError != nil {
		return nil, m.listChannelsError
	}
	return m.listChannelsResponse, nil
}

func (m *mockGroupService) ListParticipants(
	_ context.Context,
	_ *controllerapi.ListParticipantsRequest,
) (*controllerapi.ListParticipantsResponse, error) {
	if m.listParticipantsError != nil {
		return nil, m.listParticipantsError
	}
	return m.listParticipantsResponse, nil
}

func (m *mockGroupService) GetChannelDetails(
	_ context.Context,
	_ string,
) (db.Channel, error) {
	return db.Channel{}, nil
}
