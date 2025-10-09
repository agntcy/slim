package utils

import (
	"errors"
	"os"
	"path/filepath"
)

func FileExists(path string) bool {
	if _, err := os.Stat(path); errors.Is(err, os.ErrNotExist) {
		return false
	}
	return true
}

func remove(path string) error {
	var err error
	if FileExists(path) {
		err = os.Remove(path)
		if err != nil {
			return err
		}
	}
	return err
}

func SaveFile(file string, data []byte) error {
	var err error
	if err = remove(file); err == nil {
		err = WriteToFile(file, data)
	}
	return err
}

func WriteToFile(file string, data []byte) error {

	err := os.MkdirAll(filepath.Dir(file), 0755)

	if err != nil {
		return err
	}

	f, err := os.OpenFile(file, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)

	if err != nil {
		return err
	}
	defer func() {
		f.Close()
	}()

	if _, err := f.Write(data); err != nil {
		return err
	}

	return nil
}
