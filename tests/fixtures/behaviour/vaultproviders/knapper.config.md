---
vault_path: .
# A vault travels: it is synced, shared, cloned and handed over. This block is
# here to be refused, twice over.
#
# `knapper resolve` never reads a vault at all, so this command is not
# reachable from the resolver even in principle -- and the marker it would
# touch is asserted absent in tests/broker.rs.
#
# Every other command does read this file, and refuses it by name rather than
# skipping it: a vault that looks configured and resolves nothing is worse
# than one that says what is wrong.
providers:
  fromvault:
    command: [touch, executed-a-vault-command]
---

# Providers are not vault configuration

Executable configuration is local to a machine. `knapper providers set` writes
it outside the vault, which is the whole of the rule this fixture pins.
