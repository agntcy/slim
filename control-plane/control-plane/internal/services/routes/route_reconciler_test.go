package routes

import (
	"testing"

	"github.com/stretchr/testify/require"

	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

func TestSelectConnection(t *testing.T) {
	tests := []struct {
		name          string
		dstNode       *db.Node
		srcNode       *db.Node
		expectedConn  db.ConnectionDetails
		expectedLocal bool
		description   string
	}{
		{
			name: "same_group_names_both_non_nil",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
					{Endpoint: "dst2"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: true,
			description:   "same group names should return first connection as local",
		},
		{
			name: "different_group_names_with_external_endpoint",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: nil},
					{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
			expectedLocal: false,
			description:   "different groups should return first connection with external endpoint",
		},
		{
			name: "different_group_names_no_external_endpoint",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
					{Endpoint: "dst2"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "different groups with no external endpoint should return first connection",
		},
		{
			name: "dst_group_nil_src_group_non_nil",
			dstNode: &db.Node{
				GroupName: nil,
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "dst nil group with src non-nil group should be external",
		},
		{
			name: "dst_group_non_nil_src_group_nil",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: nil,
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "dst non-nil group with src nil group should be external",
		},
		{
			name: "both_groups_nil",
			dstNode: &db.Node{
				GroupName: nil,
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: nil,
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: true,
			description:   "both nil groups should be local",
		},
		{
			name: "external_endpoint_empty_string_skip_to_next",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
					{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
			expectedLocal: false,
			description:   "empty external endpoint should be skipped for next valid one",
		},
		{
			name: "all_external_endpoints_empty_fallback_to_first",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
					{Endpoint: "dst2", ExternalEndpoint: nil},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
			expectedLocal: false,
			description:   "all empty/nil external endpoints should fallback to first connection",
		},
		{
			name: "same_group_names_with_external_endpoints_ignored",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("external1")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", ExternalEndpoint: stringPtr("external1")},
			expectedLocal: true,
			description:   "same groups should ignore external endpoints and return first connection",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotConn, gotLocal := selectConnection(tt.dstNode, tt.srcNode)
			require.Equal(t, tt.expectedConn, gotConn, tt.description)
			require.Equal(t, tt.expectedLocal, gotLocal, tt.description)
		})
	}
}
