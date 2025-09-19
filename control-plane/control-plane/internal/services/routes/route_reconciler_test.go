package routes

import (
	"testing"

	"github.com/stretchr/testify/require"

	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

func TestSelectConnection(t *testing.T) {
	tests := []struct {
		name           string
		dstConnections []db.ConnectionDetails
		srcConnections []db.ConnectionDetails
		expectedConn   db.ConnectionDetails
		expectedLocal  bool
		description    string
	}{
		{
			name: "dst_no_group_name_nil",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: nil},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: nil},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: nil},
			expectedLocal: true,
			description:   "dst connection with nil group name should be considered local",
		},
		{
			name: "dst_no_group_name_empty",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("")},
				{Endpoint: "dst2", GroupName: stringPtr("group1")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group2")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("")},
			expectedLocal: true,
			description:   "dst connection with empty group name should be considered local",
		},
		{
			name: "matching_group_names",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1")},
				{Endpoint: "dst2", GroupName: stringPtr("group2")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group1")},
				{Endpoint: "src2", GroupName: stringPtr("group2")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("group1")},
			expectedLocal: true,
			description:   "connections with matching group names should be considered local",
		},
		{
			name: "no_matching_groups_with_external_endpoint",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1"), ExternalEndpoint: nil},
				{Endpoint: "dst2", GroupName: stringPtr("group1"), ExternalEndpoint: stringPtr("external1")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group2")},
				{Endpoint: "src2", GroupName: stringPtr("group2")},
			},
			expectedConn: db.ConnectionDetails{Endpoint: "dst2", GroupName: stringPtr("group1"),
				ExternalEndpoint: stringPtr("external1")},
			expectedLocal: false,
			description:   "no matching groups should return first dst connection with external endpoint",
		},
		{
			name: "no_matching_groups_no_external_endpoint",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1")},
				{Endpoint: "dst2", GroupName: stringPtr("group2")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group3")},
				{Endpoint: "src2", GroupName: stringPtr("group4")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("group1")},
			expectedLocal: false,
			description:   "no matching groups and no external endpoint should return first dst connection",
		},
		{
			name: "src_nil_group_names",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: nil},
				{Endpoint: "src2", GroupName: stringPtr("")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("group1")},
			expectedLocal: false,
			description:   "src connections with nil/empty group names don't match dst with group",
		},
		{
			name: "empty_src_connections",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1")},
			},
			srcConnections: []db.ConnectionDetails{},
			expectedConn:   db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("group1")},
			expectedLocal:  false,
			description:    "empty src connections should return first dst as external",
		},
		{
			name: "mixed_nil_empty_groups",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1")},
				{Endpoint: "dst2", GroupName: nil},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group2")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: stringPtr("group1")},
			expectedLocal: true,
			description:   "dst with nil group should make connection local",
		},
		{
			name: "external_endpoint_empty_string",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: stringPtr("group1"), ExternalEndpoint: stringPtr("")},
				{Endpoint: "dst2", GroupName: stringPtr("group2"), ExternalEndpoint: stringPtr("external2")},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group3")},
			},
			expectedConn: db.ConnectionDetails{Endpoint: "dst2", GroupName: stringPtr("group2"),
				ExternalEndpoint: stringPtr("external2")},
			expectedLocal: false,
			description:   "empty external endpoint should be skipped, use next with valid external endpoint",
		},
		{
			name: "all_dst_with_nil_group",
			dstConnections: []db.ConnectionDetails{
				{Endpoint: "dst1", GroupName: nil},
				{Endpoint: "dst2", GroupName: nil},
			},
			srcConnections: []db.ConnectionDetails{
				{Endpoint: "src1", GroupName: stringPtr("group1")},
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", GroupName: nil},
			expectedLocal: true,
			description:   "all dst with nil group should be considered local",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotConn, gotLocal := selectConnection(tt.dstConnections, tt.srcConnections)
			require.Equal(t, tt.expectedConn, gotConn, tt.description)
			require.Equal(t, tt.expectedLocal, gotLocal, tt.description)
		})
	}
}
