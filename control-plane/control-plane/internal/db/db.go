package db

type DataAccess interface {
	ListNodes() ([]Node, error)
	GetNodeByID(id string) (*Node, error)
}

type Node struct {
	ID   string
	Host string
	Port uint32
}

type dbService struct{}

func NewDBService() DataAccess {
	return &dbService{}
}

var nodes = []Node{
	{ID: "node1", Host: "127.0.0.1", Port: 46368},
	{ID: "node2", Host: "127.0.0.1", Port: 46369},
	{ID: "node3", Host: "127.0.0.1", Port: 46367},
}

func (d *dbService) ListNodes() ([]Node, error) {
	return nodes, nil
}

func (d *dbService) GetNodeByID(id string) (*Node, error) {
	for _, node := range nodes {
		if node.ID == id {
			return &node, nil
		}
	}
	return nil, nil
}
