Feature: The kernel does not link a runtime or a model client

  Scenario: Kernel and tick sources do not name a runtime or a database
    When the kernel and tick sources are scanned

  Scenario: Crate manifests do not name a model crate
    When the crate manifests are scanned
