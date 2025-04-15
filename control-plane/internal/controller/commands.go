package controller

type ControlCommand interface {
	// CommandType returns the type (subscribe, unsubscribe, etc.)
	CommandType() ControlCommandType
}

type ControlCommandType int

const (
	Subscribe ControlCommandType = iota
	Unsubscribe
)

type SubscribeCommand struct {
}

func (sc SubscribeCommand) CommandType() ControlCommandType {
	return Subscribe
}

type UnsubscribeCommand struct {
}

func (uc UnsubscribeCommand) CommandType() ControlCommandType {
	return Unsubscribe
}
