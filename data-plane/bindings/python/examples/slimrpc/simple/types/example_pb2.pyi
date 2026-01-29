from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class ExampleRequest(_message.Message):
    __slots__ = ("example_string", "example_integer")
    EXAMPLE_STRING_FIELD_NUMBER: _ClassVar[int]
    EXAMPLE_INTEGER_FIELD_NUMBER: _ClassVar[int]
    example_string: str
    example_integer: int
    def __init__(self, example_string: _Optional[str] = ..., example_integer: _Optional[int] = ...) -> None: ...

class ExampleResponse(_message.Message):
    __slots__ = ("example_string", "example_integer")
    EXAMPLE_STRING_FIELD_NUMBER: _ClassVar[int]
    EXAMPLE_INTEGER_FIELD_NUMBER: _ClassVar[int]
    example_string: str
    example_integer: int
    def __init__(self, example_string: _Optional[str] = ..., example_integer: _Optional[int] = ...) -> None: ...
