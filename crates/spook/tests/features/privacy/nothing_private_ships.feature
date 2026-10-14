Feature: Nothing private ships

  Full-history scan reports no findings. The secret-scan job in CI
  checks every commit. This feature does not run that job.

  Scenario: No tracked file contains a home-directory path
    Then no tracked file contains a home-directory path
