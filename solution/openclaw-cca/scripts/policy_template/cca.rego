package verification

predefined_values := {
    "cca_rpv": "",
    "cca_rim": "",
    "cca_rem0": "",
    "cca_rem1": "",
    "cca_rem2": "",
    "cca_rem3": ""
}

default attestation_valid = false

attestation_valid {
    input.cca.realm_token.cca_rpv == predefined_values.cca_rpv
    input.cca.realm_token.cca_rim == predefined_values.cca_rim
    input.cca.realm_token.cca_rem0 == predefined_values.cca_rem0
    input.cca.realm_token.cca_rem1 == predefined_values.cca_rem1
    input.cca.realm_token.cca_rem2 == predefined_values.cca_rem2
    input.cca.realm_token.cca_rem3 == predefined_values.cca_rem3
}

result = {
    "policy_matched": attestation_valid
}