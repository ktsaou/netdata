# 7.4 Variables or Metrics Not Found

The alert fails to load or shows errors about missing variables.

## 7.4.1 Debugging Variables

```bash
curl -s "http://localhost:19999/api/v1/alarm_variables?chart=system.cpu"
```

This shows all available variables for a chart including:
- Dimension values (stored and raw)
- Chart variables (like `update_every`, `last_collected_t`)
- Host variables
- Alert status constants (`CLEAR`, `WARNING`, `CRITICAL`, etc.)

## 7.4.2 Common Variable Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `undefined variable 'thiss'` | Typo in variable name | Correct to `$this` |
| `undefined variable 'usr'` | Dimension name doesn't exist | Use actual dimension name from `alarm_variables` API |
| Expression evaluates to NaN | Data not yet collected or dimension missing | Check data availability with the API; verify chart is collecting data |

## 7.4.3 Variable Lookup Order

When Netdata evaluates a variable, it searches in this order:

1. Built-in variables (`$this`, `$now`, `$after`, `$before`, `$status`)
2. Status constants (`CLEAR`, `WARNING`, `CRITICAL`, `UNDEFINED`, `UNINITIALIZED`, `REMOVED`)
3. Dimension values in the current chart
4. Chart variables (custom variables set on the chart)
5. Host variables (custom variables set on the host)
6. Alert values from running alerts
7. Cross-chart lookups using `context.dimension` notation

If the variable is not found in any of these locations, the expression evaluator logs an `undefined variable` error.

## 7.4.4 Related Sections

- **3.5 Variables and Special Symbols** for variable reference
- **9.3 Inspect Alert Variables** for API usage