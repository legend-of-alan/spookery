Feature: The kernel does not link a runtime or a model client

  Scenario: Kernel and tick sources do not name a runtime or a database
    When the kernel and tick sources are scanned

  Scenario: The scan fails when reqwest appears in kernel or tick
    When reqwest is planted in kernel and tick

  Scenario: Crate manifests do not name a model crate
    When the crate manifests are scanned

  Scenario: The scheduler sleeps on the calling thread
    When the scheduler source is scanned
