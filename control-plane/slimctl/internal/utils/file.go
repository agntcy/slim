package utils

import (
	"os"
	"path/filepath"

	"github.com/spf13/afero"
)

func remove(fs afero.Fs, path string) error {
	var err error
	var exists bool

	if exists, err = afero.Exists(fs, path); exists {
		err = fs.Remove(path)
	}

	return err
}

func SaveFile(fs afero.Fs, file string, data []byte) error {
	var err error
	if err = remove(fs, file); err == nil {
		err = WriteToFile(fs, file, data)
	}
	return err
}

func WriteToFile(fs afero.Fs, file string, data []byte) error {

	err := fs.MkdirAll(filepath.Dir(file), 0755)

	if err != nil {
		return err
	}

	f, err := fs.OpenFile(file, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)

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
