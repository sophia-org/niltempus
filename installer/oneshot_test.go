package main

import (
	"errors"
	"reflect"
	"strings"
	"testing"
)

// oneShotRecorder plays the installer's steps and records each call.
type oneShotRecorder struct {
	calls     []string
	failPrep  string
	failInst  bool
	failPick  string
	failBack  string
	unchanged string
}

func (r *oneShotRecorder) steps() oneShot {
	version := func(name string) ComponentVersion {
		return ComponentVersion{Name: name, SHA256: name + "-new"}
	}
	return oneShot{
		prepare: func(name, wm string) (ComponentVersion, error) {
			r.calls = append(r.calls, "prepare "+name+" wm="+wm)
			if name == r.failPrep {
				return ComponentVersion{}, errors.New("build failed")
			}
			return version(name), nil
		},
		versionPath: func(v ComponentVersion) string { return "/versions/" + v.SHA256 },
		install: func() error {
			r.calls = append(r.calls, "install")
			if r.failInst {
				return errors.New("sudo refused")
			}
			return nil
		},
		selected: func(name string) (ComponentSelection, error) {
			if name == r.unchanged {
				return ComponentSelection{Schema: 1, Current: version(name)}, nil
			}
			return ComponentSelection{Schema: 1, Current: ComponentVersion{Name: name, SHA256: name + "-old"}}, nil
		},
		choose: func(v ComponentVersion) error {
			r.calls = append(r.calls, "select "+v.Name)
			if v.Name == r.failPick {
				return errors.New("disk full")
			}
			return nil
		},
		restore: func(name string) error {
			r.calls = append(r.calls, "restore "+name)
			if name == r.failBack {
				return errors.New("no previous")
			}
			return nil
		},
	}
}

func allComponents() map[string]Repository {
	return map[string]Repository{"sophia": {}, "hagia": {}, "narthex": {}, "lom": {}, "bemenu": {}, "kleis": {}}
}

func TestOneShotPreparesEverythingBeforeInstallingThenSelects(t *testing.T) {
	r := &oneShotRecorder{}
	prepared, err := r.steps().run(allComponents())
	if err != nil {
		t.Fatal(err)
	}
	want := []string{
		"prepare hagia wm=", "prepare lom wm=/versions/hagia-new", "prepare bemenu wm=/versions/hagia-new",
		"prepare kleis wm=/versions/hagia-new", "install",
		"select hagia", "select lom", "select bemenu", "select kleis",
	}
	if !reflect.DeepEqual(r.calls, want) {
		t.Fatalf("calls %q, want %q", r.calls, want)
	}
	if len(prepared) != 4 || prepared[3].Name != "kleis" {
		t.Fatalf("prepared %+v", prepared)
	}
}

func TestOneShotPreparesOnlyConfiguredComponents(t *testing.T) {
	r := &oneShotRecorder{}
	configured := allComponents()
	delete(configured, "kleis")
	if _, err := r.steps().run(configured); err != nil {
		t.Fatal(err)
	}
	for _, call := range r.calls {
		if strings.Contains(call, "kleis") {
			t.Fatalf("unconfigured kleis was touched: %q", r.calls)
		}
	}
}

func TestOneShotPreparationFailureChangesNothing(t *testing.T) {
	for _, name := range oneShotOrder {
		r := &oneShotRecorder{failPrep: name}
		_, err := r.steps().run(allComponents())
		if err == nil || !strings.Contains(err.Error(), name+" was not prepared") || !strings.Contains(err.Error(), "nothing was installed or selected") {
			t.Fatalf("%s: %v", name, err)
		}
		for _, call := range r.calls {
			if call == "install" || strings.HasPrefix(call, "select") || strings.HasPrefix(call, "restore") {
				t.Fatalf("%s: a failed preparation went on to %q", name, call)
			}
		}
	}
}

func TestOneShotInstallFailureSelectsNothing(t *testing.T) {
	r := &oneShotRecorder{failInst: true}
	_, err := r.steps().run(allComponents())
	if err == nil || !strings.Contains(err.Error(), "desktop activation may already have occurred") {
		t.Fatal(err)
	}
	for _, call := range r.calls {
		if strings.HasPrefix(call, "select ") {
			t.Fatalf("selected after a failed installation: %q", r.calls)
		}
	}
}

func TestOneShotSelectionFailureRestoresSwitchedComponents(t *testing.T) {
	r := &oneShotRecorder{failPick: "bemenu", unchanged: "lom"}
	_, err := r.steps().run(allComponents())
	if err == nil || !strings.Contains(err.Error(), "selecting bemenu failed") || !strings.Contains(err.Error(), "restored original component state for [hagia bemenu]") {
		t.Fatal(err)
	}
	// lom was already current, so it is neither selected nor restored; kleis
	// is never reached.
	want := []string{"install", "select hagia", "select bemenu", "restore bemenu", "restore hagia"}
	if got := r.calls[len(r.calls)-len(want):]; !reflect.DeepEqual(got, want) {
		t.Fatalf("calls %q, want suffix %q", r.calls, want)
	}
}

func TestOneShotReportsAFailedRestore(t *testing.T) {
	r := &oneShotRecorder{failPick: "kleis", failBack: "lom"}
	_, err := r.steps().run(allComponents())
	if err == nil || !strings.Contains(err.Error(), "also failed") || !strings.Contains(err.Error(), "lom: no previous") {
		t.Fatal(err)
	}
	want := []string{"restore kleis", "restore bemenu", "restore lom", "restore hagia"}
	if got := r.calls[len(r.calls)-len(want):]; !reflect.DeepEqual(got, want) {
		t.Fatalf("restores %q, want %q in reverse order", r.calls, want)
	}
}

func TestOneShotProfileFailureRestoresAllAttemptedSelections(t *testing.T) {
	r := &oneShotRecorder{}
	steps := r.steps()
	steps.validate = func() error { return errors.New("profile refused") }
	_, err := steps.run(allComponents())
	if err == nil || !strings.Contains(err.Error(), "profile refused") {
		t.Fatal(err)
	}
	want := []string{"restore kleis", "restore bemenu", "restore lom", "restore hagia"}
	if got := r.calls[len(r.calls)-len(want):]; !reflect.DeepEqual(got, want) {
		t.Fatalf("restores %q, want %q", got, want)
	}
}

func TestOneShotPreflightFailureNeverActivates(t *testing.T) {
	r := &oneShotRecorder{}
	steps := r.steps()
	steps.preflight = func(v []ComponentVersion) error {
		if len(v) != 4 {
			t.Fatal("preflight before all components ready")
		}
		return errors.New("invalid combined profile")
	}
	_, err := steps.run(allComponents())
	if err == nil || !strings.Contains(err.Error(), "invalid combined profile") {
		t.Fatal(err)
	}
	for _, call := range r.calls {
		if call == "install" || strings.HasPrefix(call, "select ") {
			t.Fatalf("activated after failed preflight: %q", r.calls)
		}
	}
}
