package controlplane

import (
	"fmt"

	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func GetSubscriptionID(subscription *grpcapi.Subscription) string {
	return fmt.Sprintf("%s/%s/%s/%v->%s",
		subscription.Component_0,
		subscription.Component_1,
		subscription.Component_2,
		subscription.Id,
		subscription.ConnectionId)
}
