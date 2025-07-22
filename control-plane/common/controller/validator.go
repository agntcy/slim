package controller

import (
	_ "embed"
	"log"

	"github.com/xeipuuv/gojsonschema"
)

//go:generate cp ../../../data-plane/core/config/src/grpc/schema/client-config.schema.json ./schema.json
//go:embed schema.json
var schemaData []byte

// SchemaValidator is a global, compiled JSON schema validator.
var SchemaValidator *gojsonschema.Schema

func init() {
	loader := gojsonschema.NewBytesLoader(schemaData)
	var err error
	SchemaValidator, err = gojsonschema.NewSchema(loader)
	if err != nil {
		log.Fatalf("Failed to compile JSON schema: %v", err)
	}
}

// Validate validates input JSON bytes against the compiled schema.
func Validate(jsonData []byte) bool {
	documentLoader := gojsonschema.NewBytesLoader(jsonData)
	result, err := SchemaValidator.Validate(documentLoader)
	if err != nil {
		log.Printf("Error validating JSON: %v", err)
		return false
	}
	return result.Valid()
}
