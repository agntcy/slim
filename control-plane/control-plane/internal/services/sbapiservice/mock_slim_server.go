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
	AckConnectionError   bool
	AckSubscriptionError bool

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
	if ackMsg, err := m.stream.Recv(); err != nil {
		return fmt.Errorf("waiting initial ACK failed: %w", err)
	} else {
		if ack := ackMsg.GetAck(); ack == nil {
			return fmt.Errorf("expected initial ACK, got %T", ackMsg.Payload)
		}
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
				GroupName:        m.GroupName,
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
			fmt.Println(fmt.Sprintf("recvLoop err: %v", err))
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
		key := fmt.Sprintf("%s/%s/%s/%d", c.Component_0, c.Component_1, c.Component_2, c.GetId().GetValue())
		m.recvSubscriptions[key] = c
	}
	for _, c := range cfg.SubscriptionsToDelete {
		if m.recvSubscriptions != nil {
			key := fmt.Sprintf("%s/%s/%s/%d", c.Component_0, c.Component_1, c.Component_2, c.GetId().GetValue())
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
	if m.AckConnectionError {
		for _, c := range cfg.ConnectionsToCreate {
			ack.GetConfigCommandAck().ConnectionsFailedToCreate = append(ack.GetConfigCommandAck().ConnectionsFailedToCreate, &controllerapi.ConnectionError{
				ConnectionId: c.ConnectionId,
				ErrorMsg:     "conn error",
			})
		}
	}
	if m.AckSubscriptionError {
		for _, s := range cfg.SubscriptionsToSet {
			ack.GetConfigCommandAck().SubscriptionsFailedToSet = append(ack.GetConfigCommandAck().SubscriptionsFailedToSet, &controllerapi.SubscriptionError{
				Subscription: s,
				ErrorMsg:     "sub error",
			})
		}
	}
	err := m.stream.Send(ack)
	if err != nil {
		fmt.Printf("handleConfigCommand: send ACK error: %v\n", err)
	}
}

// updateSubscription sends a single subscription config command (add or delete when delete=true).
func (m *MockSlimServer) updateSubscription(ctx context.Context, comp0, comp1, comp2 string, id int, delete bool) error {
	payload := &controllerapi.ConfigurationCommand{}
	if delete {
		payload.SubscriptionsToDelete = []*controllerapi.Subscription{{
			Component_0:  comp0,
			Component_1:  comp1,
			Component_2:  comp2,
			Id:           wrapperspb.UInt64(uint64(id)),
			ConnectionId: "n/a",
		}}
	} else {
		payload.SubscriptionsToSet = []*controllerapi.Subscription{{
			Component_0:  comp0,
			Component_1:  comp1,
			Component_2:  comp2,
			Id:           wrapperspb.UInt64(uint64(id)),
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

func (m *MockSlimServer) GetReceived() (conns map[string]*controllerapi.Connection, subs map[string]*controllerapi.Subscription) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.recvConnections, m.recvSubscriptions
}
