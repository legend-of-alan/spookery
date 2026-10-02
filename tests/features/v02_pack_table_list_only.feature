@v02
Feature: Pack-table list only
  Proof: v02_p02_pack_table_stdio_does_not_gain_say_or_a_tools_call_body
  spook run --mcp pack-table does not gain say and does not gain a tools/call body.

  Scenario: Pack-table stdio does not gain say or a tools/call body
    Given a temp copy of pack packs/oneshot-docks
    When pack-table stdio lists tools and calls say and session_status
    Then pack-table tools/list names are the recorded table
    And pack-table tools/list omits say
    And every pack-table tool input schema is type object
    And pack-table tools/call say is METHOD_NOT_FOUND
    And pack-table tools/call session_status is METHOD_NOT_FOUND
    And pack-table leaves state/now.yaml unchanged
    And pack-table leaves state/log/session.jsonl absent
