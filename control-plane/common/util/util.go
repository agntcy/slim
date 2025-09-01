package util

import (
	"fmt"
	"strconv"
	"strings"
)

func ParseRoute(route string) (
	organization,
	namespace,
	appName string,
	appInstance uint64,
	err error,
) {
	parts := strings.Split(route, "/")

	if len(parts) != 4 {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/appname/appinstance'",
			route,
		)
		return
	}

	if parts[0] == "" || parts[1] == "" || parts[2] == "" || parts[3] == "" {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/appname/appinstance'",
			route,
		)
		return
	}

	organization = parts[0]
	namespace = parts[1]
	appName = parts[2]

	appInstance, err = strconv.ParseUint(parts[3], 10, 64)
	if err != nil {
		err = fmt.Errorf("invalid app instance ID (must be u64) %s", parts[3])
		return
	}

	return
}
