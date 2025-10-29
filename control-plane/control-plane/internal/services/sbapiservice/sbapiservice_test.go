package sbapiservice

import (
	"context"
	"fmt"
	"net"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// startSouthbound spins up a grpc server with Southbound API and returns listen target.
func startSouthbound(t *testing.T, db db.DataAccess) (target string, cleanup func()) {
	// t.Helper()

	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{Level: "debug"})
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	routeService := routes.NewRouteService(db, cmdHandler,
		config.ReconcilerConfig{MaxNumOfParallelReconciles: 3, MaxRequeues: 1})
	if err := routeService.Start(ctx); err != nil {
		t.Fatalf("routeService.Start: %v", err)
	}
	grp := groupservice.NewGroupService(db, cmdHandler)

	s := grpc.NewServer(grpc.Creds(insecure.NewCredentials()))
	svc := NewSBAPIService(config.APIConfig{
		HTTPHost: "127.0.0.1",
		HTTPPort: "50052",
	}, config.LogConfig{Level: "debug"}, db, cmdHandler, routeService, grp)
	controllerapi.RegisterControllerServiceServer(s, svc)
	l, err := net.Listen("tcp", "127.0.0.1:50052")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	go func() { _ = s.Serve(l) }()
	return l.Addr().String(), func() { s.Stop(); l.Close(); routeService.Stop() }
}

func waitCond(t *testing.T, d time.Duration, cond func() bool, msg string) {
	fmt.Println(msg)
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		time.Sleep(1 * time.Second)
		if cond() {
			return
		}
	}
	t.Fatalf("condition `%s` not met within %s", msg, d)
}

func TestSouthbound_RegistrationAndRouteHandling(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	// start three mock nodes
	slim0, _ := NewMockSlimServer("slim-0", 4500, target)
	slim1, _ := NewMockSlimServer("slim-1", 4501, target)
	slim2, _ := NewMockSlimServer("slim-2", 4502, target)

	ctx := context.Background()
	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim0 start: %v", err)
	}
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim1 start: %v", err)
	}
	if err := slim2.Start(ctx); err != nil {
		t.Fatalf("slim2 start: %v", err)
	}

	// registration reaches DB
	time.Sleep(3 * time.Second)
	require.Len(t, db.ListNodes(), 3)

	// slim-0 publishes a subscription org/test/client/0
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, false); err != nil {
		t.Fatalf("failed to send subcription update: %v", err)
	}

	// check that routes created in DB: from other nodes to slim-0
	waitCond(t, 3*time.Second, func() bool {
		for _, r := range db.GetRoutesForDestinationNodeID("slim-0") {
			if r.DestNodeID == "slim-0" && r.Component0 == "org" && r.Component2 == "client" {

				return true
			}
		}
		return false
	}, "wait for route for slim-0 to be created")

	// other instances should receive connections+subscriptions for slim-0
	waitCond(t, 3*time.Second, func() bool {
		_, subs1 := slim1.GetReceived()
		_, subs2 := slim2.GetReceived()
		return len(subs1) > 0 && len(subs2) > 0
	}, "wait for subs to be received by slim-1 and slim-2")

	// restart slim-1 and expect it to receive the same config
	_ = slim1.Close()
	slim1, _ = NewMockSlimServer("slim-1", 4501, target)
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim-1 connect: %v", err)
	}
	waitCond(t, 3*time.Second, func() bool {
		_, subs := slim1.GetReceived()
		return len(subs) > 0
	}, "wait for subscriptions to be received by slim-1")

	// restart slim-0 with different port; expect reconcilers update other nodes
	_ = slim0.Close()
	slim0, _ = NewMockSlimServer("slim-0", 4800, target)
	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim-0 connect: %v", err)
	}

	// other instances should receive conns+subs for slim-0
	waitCond(t, 3*time.Second, func() bool {
		foundOnSlim0, foundOnSlim1 := false, false
		conns1, _ := slim1.GetReceived()
		for _, c := range conns1 {
			if c.ConnectionId == "http://127.0.0.1:4800" {
				foundOnSlim0 = true
				break
			}
		}
		conns2, _ := slim2.GetReceived()
		for _, c := range conns2 {
			if c.ConnectionId == "http://127.0.0.1:4800" {
				foundOnSlim1 = true
				break
			}
		}
		return foundOnSlim0 && foundOnSlim1
	}, "wait for changed subs to be received by slim-1 and slim-2")

	// send delete for subscription
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, true); err != nil {
		t.Fatalf("delete sub: %v", err)
	}

	// routes should be removed
	waitCond(t, 3*time.Second, func() bool {
		// route for slim-0 should be gone
		rs := db.GetRoutesForDestinationNodeID("slim-0")
		if len(rs) != 0 {
			return false
		}
		_, subs1 := slim1.GetReceived()
		_, subs2 := slim2.GetReceived()
		return len(subs1) == 0 && len(subs2) == 0
	}, "wait for route for slim-0 to be deleted")

	_ = slim2.Close()
	_ = slim1.Close()
	_ = slim0.Close()
}

func TestSouthbound_RouteWithConnectionError(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	slim0, _ := NewMockSlimServer("slim-0", 4500, target)
	slim1, _ := NewMockSlimServer("slim-1", 4501, target)
	slim1.AckConnectionError = true
	slim1.AckSubscriptionSetError = true

	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim0 start: %v", err)
	}
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim1 start: %v", err)
	}

	// wait nodes in DB
	waitCond(t, 2*time.Second, func() bool { return len(db.ListNodes()) == 2 },
		"wait for 2 nodes to register")

	// slim-0 sends subscription
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, false); err != nil {
		t.Fatalf("send sub: %v", err)
	}

	// wait reconciler to mark routes for slim-1 as failed
	waitCond(t, 2*time.Second, func() bool {
		for _, r := range db.GetRoutesForNodeID("slim-1") {
			if r.SourceNodeID == "slim-1" && r.DestNodeID == "slim-0" && r.StatusMsg != "" {
				return true
			}
		}
		return false
	}, "wait for route slim-1:org/test/client/->slim-0 to be marked as failed")

	_ = slim0.Close()
	_ = slim1.Close()
}
