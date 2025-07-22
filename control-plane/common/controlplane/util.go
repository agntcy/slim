package controlplane

import (
	"fmt"

	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func GetSubscriptionID(subscription *grpcapi.Subscription) string {
	return fmt.Sprintf("%s/%s/%s/%v->%s",
		subscription.Organization,
		subscription.Namespace,
		subscription.AgentType,
		subscription.AgentId,
		subscription.ConnectionId)
}
