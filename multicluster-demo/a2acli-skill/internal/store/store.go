// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package store manages the local directory of A2A AgentCard JSON files.
package store

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/a2aproject/a2a-go/v2/a2a"
)

const agentsDirName = ".a2aagents"

// Agent holds a parsed AgentCard alongside its digest.
type Agent struct {
	// Digest is the full SHA256 digest of the raw AgentCard file, e.g. "sha256:3f7a2c1b...".
	Digest string
	// Card is the parsed AgentCard.
	Card *a2a.AgentCard
}

// ShortDigest returns the short form of the digest suitable for display, e.g. "sha256:3f7a2c1b...".
func (a *Agent) ShortDigest() string {
	parts := strings.SplitN(a.Digest, ":", 2)
	if len(parts) != 2 || len(parts[1]) < 8 {
		return a.Digest
	}
	return parts[0] + ":" + parts[1][:8] + "..."
}

// Store provides access to the local agent directory.
type Store struct {
	// Dir is the resolved path to the .a2aagents directory.
	Dir string
}

// NewStore resolves and returns a Store, searching for the agents directory in this order:
//  1. explicitDir if non-empty
//  2. ./.a2aagents in the current working directory
//  3. ~/.a2aagents in the user's home directory
func NewStore(explicitDir string) (*Store, error) {
	if explicitDir != "" {
		info, err := os.Stat(explicitDir)
		if err != nil {
			return nil, fmt.Errorf("agents directory %q not found: %w", explicitDir, err)
		}
		if !info.IsDir() {
			return nil, fmt.Errorf("%q is not a directory", explicitDir)
		}
		return &Store{Dir: explicitDir}, nil
	}

	// Try ./.a2aagents
	localDir := filepath.Join(".", agentsDirName)
	if info, err := os.Stat(localDir); err == nil && info.IsDir() {
		return &Store{Dir: localDir}, nil
	}

	// Try ~/.a2aagents
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("could not determine home directory: %w", err)
	}
	homeDir := filepath.Join(home, agentsDirName)
	if info, err := os.Stat(homeDir); err == nil && info.IsDir() {
		return &Store{Dir: homeDir}, nil
	}

	return nil, fmt.Errorf("no agents directory found; create %q in the current directory or %q in your home directory", localDir, homeDir)
}

// List returns all agents in the store, sorted by name.
func (s *Store) List() ([]*Agent, error) {
	entries, err := os.ReadDir(s.Dir)
	if err != nil {
		return nil, fmt.Errorf("reading agents directory %q: %w", s.Dir, err)
	}

	var agents []*Agent
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		agent, err := s.readAgent(filepath.Join(s.Dir, entry.Name()))
		if err != nil {
			// Skip files that can't be parsed, but don't fail the whole list.
			fmt.Fprintf(os.Stderr, "warning: skipping %s: %v\n", entry.Name(), err)
			continue
		}
		agents = append(agents, agent)
	}

	sort.Slice(agents, func(i, j int) bool {
		return agents[i].Card.Name < agents[j].Card.Name
	})

	return agents, nil
}

// Lookup finds a single agent by its full digest or an unambiguous prefix.
// Returns an error if zero or multiple agents match.
func (s *Store) Lookup(digestOrPrefix string) (*Agent, error) {
	agents, err := s.List()
	if err != nil {
		return nil, err
	}

	var matches []*Agent
	for _, agent := range agents {
		if strings.HasPrefix(agent.Digest, digestOrPrefix) {
			matches = append(matches, agent)
		}
	}

	switch len(matches) {
	case 0:
		return nil, fmt.Errorf("no agent found matching digest %q", digestOrPrefix)
	case 1:
		return matches[0], nil
	default:
		names := make([]string, len(matches))
		for i, m := range matches {
			names[i] = fmt.Sprintf("%s (%s)", m.Card.Name, m.ShortDigest())
		}
		return nil, fmt.Errorf("ambiguous digest prefix %q matches multiple agents: %s", digestOrPrefix, strings.Join(names, ", "))
	}
}

// readAgent reads and parses an AgentCard JSON file, computing its SHA256 digest.
func (s *Store) readAgent(path string) (*Agent, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading file: %w", err)
	}

	var card a2a.AgentCard
	if err := json.Unmarshal(data, &card); err != nil {
		return nil, fmt.Errorf("parsing AgentCard JSON: %w", err)
	}

	sum := sha256.Sum256(data)
	digest := "sha256:" + hex.EncodeToString(sum[:])

	return &Agent{
		Digest: digest,
		Card:   &card,
	}, nil
}
