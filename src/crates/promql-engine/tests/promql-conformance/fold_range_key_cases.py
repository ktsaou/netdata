# SPDX-License-Identifier: GPL-3.0-or-later
"""Precise range-key witnesses, broad cache controls and retained filter workloads."""
from copy import deepcopy
from fold_geometry_cases import geometry_cases,geometry_performance_cases
from observed_filter_cases import observed_filter_performance_cases
from native_endpoint_cases import _records
from single_reset_cases import _request


def precise_range_cases():
    result=[]
    for pattern in ('float','native'):
        data=_records(14,8,pattern)
        for row in data['series']:
            for point in row['points']:
                point['time_ms']//=10
        for function in ('rate','increase','delta'):
            for duration in ('333ms','1s / 3'):
                for mode in ('instant','range'):
                    for profile in ('server-default','upstream-tests'):
                        ident=f'range-key-{pattern}-{function}-{duration}-{mode}-{profile}'
                        request=_request(ident,f'{function}(m_total[{duration}])',1,mode,profile)
                        request.update(time_ms=1000,start_ms=1000,end_ms=1200,step_ms=100)
                        result.append({'shape':'control','dataset':data,'request':request})
        if pattern=='float':
            for duration in ('333ms','1s / 3'):
                for profile in ('server-default','upstream-tests'):
                    request=_request(f'range-key-threshold-{duration}-{profile}',f'rate(m_total[{duration}]) >= bool 9.995',1,'instant',profile)
                    request.update(time_ms=1000,start_ms=1000,end_ms=1200,step_ms=100)
                    result.append({'shape':'control','dataset':data,'request':request})
    assert len(result)==52
    return result


def filter_controls():
    controls=deepcopy(observed_filter_performance_cases())
    for case in controls:case['shape']='control'
    return controls


def range_key_cases():
    return geometry_cases()+filter_controls()+precise_range_cases()+numeric_range_cases()


def range_key_performance_cases():
    return geometry_performance_cases()+filter_controls()


def numeric_range_cases():
    cases=[]
    for case in precise_range_cases():
        if '1s / 3'not in case['request']['query']:continue
        copy=deepcopy(case)
        copy['request']['query']=copy['request']['query'].replace('1s / 3','0.333333333')
        copy['request']['id']=copy['request']['id'].replace('1s / 3','0.333333333')
        cases.append(copy)
    assert len(cases)==26
    return cases
