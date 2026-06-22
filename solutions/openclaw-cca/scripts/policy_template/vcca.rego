package verification

predefined_values := {
    "vcca_rpv": "",
    "vcca_rim": "",
    "vcca_rem0": "",
    "vcca_rem1": "",
    "vcca_rem2": "",
    "vcca_rem3": ""
}

default attestation_valid = false

attestation_valid {
    input.virt_cca.realm_token.vcca_rpv == predefined_values.vcca_rpv
    input.virt_cca.realm_token.vcca_rim == predefined_values.vcca_rim
    input.virt_cca.realm_token.vcca_rem0 == predefined_values.vcca_rem0
    input.virt_cca.realm_token.vcca_rem1 == predefined_values.vcca_rem1
    input.virt_cca.realm_token.vcca_rem2 == predefined_values.vcca_rem2
    input.virt_cca.realm_token.vcca_rem3 == predefined_values.vcca_rem3
}

result = {
    "policy_matched": attestation_valid
}