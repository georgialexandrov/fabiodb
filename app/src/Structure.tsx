import type { RelationRef, TableInfo } from "./api";

type Props = { info: TableInfo; relation: RelationRef; onOpen: (r: RelationRef) => void };

export function Structure({ info, onOpen }: Props) {
  return (
    <div className="structure">
      <h3>Columns</h3>
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>Type</th>
            <th>Nullable</th>
            <th>Default</th>
          </tr>
        </thead>
        <tbody>
          {info.columns.map((c) => (
            <tr key={c.name}>
              <td>
                {c.name} {c.primary_key && <span className="badge">PK</span>}
              </td>
              <td className="muted">{c.data_type}</td>
              <td>{c.nullable ? "yes" : ""}</td>
              <td className="muted">{c.default}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {info.indexes.length > 0 && (
        <>
          <h3>Indexes</h3>
          <table>
            <tbody>
              {info.indexes.map((i) => (
                <tr key={i.name}>
                  <td>{i.name}</td>
                  <td className="muted">{i.columns.join(", ")}</td>
                  <td>{i.primary ? <span className="badge">PRIMARY</span> : i.unique && <span className="badge">UNIQUE</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      {info.foreign_keys.length > 0 && (
        <>
          <h3>Foreign keys</h3>
          <table>
            <tbody>
              {info.foreign_keys.map((f, n) => (
                <tr key={f.name ?? n}>
                  <td>{f.columns.join(", ")}</td>
                  <td>
                    →{" "}
                    <a onClick={() => onOpen({ schema: f.ref_schema, name: f.ref_table })}>
                      {f.ref_table}
                    </a>
                    <span className="muted"> ({f.ref_columns.join(", ")})</span>
                  </td>
                  <td className="muted">{f.name}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}
