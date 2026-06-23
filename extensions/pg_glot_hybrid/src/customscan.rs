use pgrx::pg_sys;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

const CUSTOM_NAME: &[u8] = b"GlotHybrid\0";
const ID_COLUMN: &[u8] = b"id\0";
const DEFAULT_K: i32 = 60;
const DEFAULT_PER_LEG: i32 = 60;

#[repr(transparent)]
struct PgStatic<T>(T);

unsafe impl<T> Sync for PgStatic<T> {}

#[repr(C)]
struct GlotHybridScanState {
    css: pg_sys::CustomScanState,
    runtime: *mut RuntimeState,
}

struct RuntimeState {
    rows: Vec<pg_sys::HeapTuple>,
    next: usize,
}

static PATH_METHODS: PgStatic<pg_sys::CustomPathMethods> = PgStatic(pg_sys::CustomPathMethods {
    CustomName: CUSTOM_NAME.as_ptr().cast::<c_char>(),
    PlanCustomPath: Some(plan_custom_path),
    ReparameterizeCustomPathByChild: None,
});

static SCAN_METHODS: PgStatic<pg_sys::CustomScanMethods> = PgStatic(pg_sys::CustomScanMethods {
    CustomName: CUSTOM_NAME.as_ptr().cast::<c_char>(),
    CreateCustomScanState: Some(create_custom_scan_state),
});

static EXEC_METHODS: PgStatic<pg_sys::CustomExecMethods> = PgStatic(pg_sys::CustomExecMethods {
    CustomName: CUSTOM_NAME.as_ptr().cast::<c_char>(),
    BeginCustomScan: Some(begin_custom_scan),
    ExecCustomScan: Some(exec_custom_scan),
    EndCustomScan: Some(end_custom_scan),
    ReScanCustomScan: Some(rescan_custom_scan),
    MarkPosCustomScan: None,
    RestrPosCustomScan: None,
    EstimateDSMCustomScan: None,
    InitializeDSMCustomScan: None,
    ReInitializeDSMCustomScan: None,
    InitializeWorkerCustomScan: None,
    ShutdownCustomScan: None,
    ExplainCustomScan: Some(explain_custom_scan),
});

static mut PREV_SET_REL_PATHLIST_HOOK: pg_sys::set_rel_pathlist_hook_type = None;

pub(crate) fn init() {
    // SAFETY: `_PG_init` runs while PostgreSQL is loading the extension in a
    // single backend. Hook chaining follows PostgreSQL's documented global hook
    // pattern: remember the previous hook, then install this extension's hook.
    unsafe {
        pg_sys::RegisterCustomScanMethods(&SCAN_METHODS.0);
        PREV_SET_REL_PATHLIST_HOOK = pg_sys::set_rel_pathlist_hook;
        pg_sys::set_rel_pathlist_hook = Some(set_rel_pathlist);
    }
}

#[pgrx::pg_guard]
unsafe extern "C-unwind" fn set_rel_pathlist(
    root: *mut pg_sys::PlannerInfo,
    rel: *mut pg_sys::RelOptInfo,
    rti: pg_sys::Index,
    rte: *mut pg_sys::RangeTblEntry,
) {
    if let Some(prev) = PREV_SET_REL_PATHLIST_HOOK {
        // SAFETY: PostgreSQL passes the hook arguments; forwarding them to the
        // next hook preserves the normal hook chain contract.
        unsafe { prev(root, rel, rti, rte) };
    }

    // SAFETY: All raw planner pointers are owned by PostgreSQL for the duration
    // of this hook invocation. `try_build_path_config` only reads planner nodes
    // after checking for null and node tags, and returns `None` for unsupported
    // query shapes so the normal planner remains available.
    let Some(config) = (unsafe { try_build_path_config(root, rel, rti, rte) }) else {
        return;
    };

    // SAFETY: `palloc0` allocates in the planner memory context. The path is
    // initialized with PostgreSQL node tags and points only at planner-owned
    // structures whose lifetime covers planning.
    unsafe {
        let custom_path =
            pg_sys::palloc0(std::mem::size_of::<pg_sys::CustomPath>()).cast::<pg_sys::CustomPath>();
        if custom_path.is_null() {
            return;
        }

        (*custom_path).path.type_ = pg_sys::NodeTag::T_CustomPath;
        (*custom_path).path.pathtype = pg_sys::NodeTag::T_CustomScan;
        (*custom_path).path.parent = rel;
        (*custom_path).path.pathtarget = (*rel).reltarget;
        (*custom_path).path.param_info = ptr::null_mut();
        (*custom_path).path.parallel_aware = false;
        (*custom_path).path.parallel_safe = false;
        (*custom_path).path.parallel_workers = 0;
        (*custom_path).path.rows = f64::from(config.limit_rows);
        (*custom_path).path.startup_cost = 0.0;
        (*custom_path).path.total_cost = 0.001;
        (*custom_path).path.pathkeys = (*root).sort_pathkeys;
        (*custom_path).flags = 0;
        (*custom_path).custom_paths = ptr::null_mut();
        (*custom_path).custom_restrictinfo = ptr::null_mut();
        (*custom_path).custom_private = make_private_list(&config.sql);
        (*custom_path).methods = &PATH_METHODS.0;

        if !(*custom_path).custom_private.is_null() {
            pg_sys::add_path(rel, custom_path.cast::<pg_sys::Path>());
        }
    }
}

#[derive(Debug)]
struct PathConfig {
    sql: String,
    limit_rows: i32,
}

unsafe fn try_build_path_config(
    root: *mut pg_sys::PlannerInfo,
    rel: *mut pg_sys::RelOptInfo,
    rti: pg_sys::Index,
    rte: *mut pg_sys::RangeTblEntry,
) -> Option<PathConfig> {
    if root.is_null() || rel.is_null() || rte.is_null() {
        return None;
    }

    if (*rel).reloptkind != pg_sys::RelOptKind::RELOPT_BASEREL {
        return None;
    }
    if !(*rel).baserestrictinfo.is_null() {
        return None;
    }
    if (*rte).rtekind != pg_sys::RTEKind::RTE_RELATION {
        return None;
    }

    let query = (*root).parse;
    if query.is_null() {
        return None;
    }
    if pg_sys::list_length((*query).rtable) != 1 {
        return None;
    }
    if !(*query).limitOffset.is_null() {
        return None;
    }
    if (*query).limitCount.is_null() {
        return None;
    }
    if (*query).limitOption != pg_sys::LimitOption::LIMIT_OPTION_COUNT {
        return None;
    }
    if pg_sys::list_length((*query).sortClause) != 1 {
        return None;
    }

    let limit_rows = limit_rows(root)?;
    let sort_clause = pg_sys::list_nth((*query).sortClause, 0).cast::<pg_sys::SortGroupClause>();
    if sort_clause.is_null() || !is_desc_sort((*sort_clause).sortop) {
        return None;
    }

    let sort_expr = pg_sys::get_sortgroupclause_expr(sort_clause, (*query).targetList);
    if sort_expr.is_null() || (*sort_expr).type_ != pg_sys::NodeTag::T_FuncExpr {
        return None;
    }

    let func = sort_expr.cast::<pg_sys::FuncExpr>();
    if !is_glot_rank((*func).funcid) {
        return None;
    }

    let nargs = pg_sys::list_length((*func).args);
    if !(nargs == 4 || nargs == 6) {
        return None;
    }

    let text_arg = pg_sys::list_nth((*func).args, 0).cast::<pg_sys::Node>();
    let vec_arg = pg_sys::list_nth((*func).args, 1).cast::<pg_sys::Node>();
    let q_text_arg = pg_sys::list_nth((*func).args, 2).cast::<pg_sys::Node>();
    let q_vec_arg = pg_sys::list_nth((*func).args, 3).cast::<pg_sys::Node>();

    let text_col = column_name_for_var((*rte).relid, rti, text_arg)?;
    let vec_col = column_name_for_var((*rte).relid, rti, vec_arg)?;
    let q_text = const_to_quoted_literal(q_text_arg)?;
    let q_vec = const_to_quoted_literal(q_vec_arg)?;
    let k = if nargs == 6 {
        const_i32(pg_sys::list_nth((*func).args, 4).cast::<pg_sys::Node>())?
    } else {
        DEFAULT_K
    };
    let per_leg = if nargs == 6 {
        const_i32(pg_sys::list_nth((*func).args, 5).cast::<pg_sys::Node>())?
    } else {
        DEFAULT_PER_LEG
    };
    if k <= 0 || per_leg <= 0 {
        return None;
    }

    let id_attno = pg_sys::get_attnum((*rte).relid, ID_COLUMN.as_ptr().cast::<c_char>());
    if id_attno <= 0 {
        return None;
    }

    let rel_sql = qualified_relation_name((*rte).relid)?;
    let id_ident = quote_identifier_sql("id")?;
    let text_col_lit = quote_literal_sql(&text_col)?;
    let vec_col_lit = quote_literal_sql(&vec_col)?;
    let rel_oid = (*rte).relid.to_u32();

    let sql = format!(
        "SELECT d.* \
         FROM {rel_sql} AS d \
         JOIN glot.hybrid({rel_oid}::oid::regclass, 'id', {text_col_lit}, {vec_col_lit}, \
                          {q_text}, {q_vec}::vector, {k}, {per_leg}, {limit_rows}) AS h \
           ON d.{id_ident} = h.id \
         ORDER BY h.score DESC, d.{id_ident}"
    );

    Some(PathConfig { sql, limit_rows })
}

unsafe extern "C-unwind" fn plan_custom_path(
    _root: *mut pg_sys::PlannerInfo,
    rel: *mut pg_sys::RelOptInfo,
    best_path: *mut pg_sys::CustomPath,
    tlist: *mut pg_sys::List,
    clauses: *mut pg_sys::List,
    custom_plans: *mut pg_sys::List,
) -> *mut pg_sys::Plan {
    if rel.is_null() || best_path.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: PostgreSQL calls this callback while converting the planner-owned
    // `CustomPath` into a plan. The allocated `CustomScan` is zeroed, tagged,
    // and populated with callback-owned private data made from serializable
    // PostgreSQL `String` nodes.
    unsafe {
        let scan =
            pg_sys::palloc0(std::mem::size_of::<pg_sys::CustomScan>()).cast::<pg_sys::CustomScan>();
        if scan.is_null() {
            return ptr::null_mut();
        }

        (*scan).scan.plan.type_ = pg_sys::NodeTag::T_CustomScan;
        (*scan).scan.plan.targetlist = tlist;
        (*scan).scan.plan.qual = clauses;
        (*scan).scan.scanrelid = (*rel).relid;
        (*scan).flags = (*best_path).flags;
        (*scan).custom_plans = custom_plans;
        (*scan).custom_exprs = ptr::null_mut();
        (*scan).custom_private =
            pg_sys::copyObjectImpl((*best_path).custom_private.cast()).cast::<pg_sys::List>();
        (*scan).custom_scan_tlist = ptr::null_mut();
        (*scan).custom_relids = ptr::null_mut();
        (*scan).methods = &SCAN_METHODS.0;

        scan.cast::<pg_sys::Plan>()
    }
}

unsafe extern "C-unwind" fn create_custom_scan_state(
    _cscan: *mut pg_sys::CustomScan,
) -> *mut pg_sys::Node {
    // SAFETY: PostgreSQL requires the provider to palloc a zeroed state node
    // whose first field is `CustomScanState`, then set the node tag and method
    // table. `GlotHybridScanState` embeds it as the first field.
    unsafe {
        let state = pg_sys::palloc0(std::mem::size_of::<GlotHybridScanState>())
            .cast::<GlotHybridScanState>();
        if state.is_null() {
            return ptr::null_mut();
        }
        (*state).css.ss.ps.type_ = pg_sys::NodeTag::T_CustomScanState;
        (*state).css.methods = &EXEC_METHODS.0;
        (*state).css.slotOps = &pg_sys::TTSOpsHeapTuple;
        (*state).runtime = ptr::null_mut();
        state.cast::<pg_sys::Node>()
    }
}

unsafe extern "C-unwind" fn begin_custom_scan(
    node: *mut pg_sys::CustomScanState,
    _estate: *mut pg_sys::EState,
    _eflags: std::os::raw::c_int,
) {
    if node.is_null() {
        pgrx::error!("glot.rank custom scan: null CustomScanState");
    }

    // SAFETY: The executor has initialized `node->ss.ps.plan` from the
    // `CustomScan` plan before invoking this callback. The first private entry
    // is the SQL string produced by `try_build_path_config`.
    let sql = unsafe {
        let plan = (*node).ss.ps.plan.cast::<pg_sys::CustomScan>();
        if plan.is_null() {
            pgrx::error!("glot.rank custom scan: missing CustomScan plan");
        }
        private_sql((*plan).custom_private)
            .unwrap_or_else(|| pgrx::error!("glot.rank custom scan: missing private SQL"))
    };

    let runtime = match execute_candidate_query(&sql) {
        Ok(runtime) => runtime,
        Err(message) => pgrx::error!("glot.rank custom scan: {message}"),
    };

    // SAFETY: `node` is really a `GlotHybridScanState` because
    // `create_custom_scan_state` allocated that exact struct with
    // `CustomScanState` at offset zero.
    unsafe {
        let state = node.cast::<GlotHybridScanState>();
        (*state).runtime = Box::into_raw(Box::new(runtime));
    }
}

unsafe extern "C-unwind" fn exec_custom_scan(
    node: *mut pg_sys::CustomScanState,
) -> *mut pg_sys::TupleTableSlot {
    if node.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: Delegating through `ExecScan` lets PostgreSQL apply the plan's
    // qual/projection using the scan slot initialized by `ExecInitCustomScan`.
    unsafe {
        pg_sys::ExecScan(
            &mut (*node).ss,
            Some(next_candidate),
            Some(recheck_candidate),
        )
    }
}

unsafe extern "C-unwind" fn end_custom_scan(node: *mut pg_sys::CustomScanState) {
    if node.is_null() {
        return;
    }

    // SAFETY: `runtime` was created with `Box::into_raw` in BeginCustomScan and
    // is owned exclusively by this scan state. Each tuple was copied out of SPI
    // with `SPI_copytuple` and is freed with the matching `SPI_freetuple`.
    unsafe {
        let state = node.cast::<GlotHybridScanState>();
        if (*state).runtime.is_null() {
            return;
        }
        let runtime = Box::from_raw((*state).runtime);
        for tuple in runtime.rows {
            if !tuple.is_null() {
                pg_sys::SPI_freetuple(tuple);
            }
        }
        (*state).runtime = ptr::null_mut();
    }
}

unsafe extern "C-unwind" fn rescan_custom_scan(node: *mut pg_sys::CustomScanState) {
    if node.is_null() {
        return;
    }

    // SAFETY: `runtime` is owned by this scan state for the executor lifetime.
    unsafe {
        let state = node.cast::<GlotHybridScanState>();
        if !(*state).runtime.is_null() {
            (*(*state).runtime).next = 0;
        }
    }
}

unsafe extern "C-unwind" fn next_candidate(
    scan_state: *mut pg_sys::ScanState,
) -> *mut pg_sys::TupleTableSlot {
    if scan_state.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: `scan_state` points at the first field of our custom state
    // (`CustomScanState.ss`). Because both embedding structs place their first
    // field at offset zero, the cast recovers `GlotHybridScanState`.
    unsafe {
        let state = scan_state.cast::<GlotHybridScanState>();
        let runtime = (*state).runtime;
        if runtime.is_null() {
            return ptr::null_mut();
        }
        let runtime = &mut *runtime;
        if runtime.next >= runtime.rows.len() {
            return ptr::null_mut();
        }

        let tuple = runtime.rows[runtime.next];
        runtime.next += 1;
        if tuple.is_null() {
            return ptr::null_mut();
        }

        pg_sys::ExecStoreHeapTuple(tuple, (*scan_state).ss_ScanTupleSlot, false)
    }
}

unsafe extern "C-unwind" fn recheck_candidate(
    _scan_state: *mut pg_sys::ScanState,
    _slot: *mut pg_sys::TupleTableSlot,
) -> bool {
    true
}

unsafe extern "C-unwind" fn explain_custom_scan(
    node: *mut pg_sys::CustomScanState,
    _ancestors: *mut pg_sys::List,
    es: *mut pg_sys::ExplainState,
) {
    if node.is_null() || es.is_null() {
        return;
    }

    // SAFETY: The executor sets `node->ss.ps.plan` to our `CustomScan` plan
    // before EXPLAIN walks it. The first private entry is the candidate SQL
    // produced by `try_build_path_config` (same accessor as BeginCustomScan).
    // `ExplainPropertyText` copies the value, so the temporary `CString` is
    // safe to drop afterwards.
    unsafe {
        let plan = (*node).ss.ps.plan.cast::<pg_sys::CustomScan>();
        if plan.is_null() {
            return;
        }
        let Some(sql) = private_sql((*plan).custom_private) else {
            return;
        };
        let (Ok(label), Ok(value)) = (CString::new("Hybrid Query"), CString::new(sql)) else {
            return;
        };
        pg_sys::ExplainPropertyText(label.as_ptr(), value.as_ptr(), es);
    }
}

fn execute_candidate_query(sql: &str) -> Result<RuntimeState, String> {
    let c_sql =
        CString::new(sql).map_err(|_| "candidate SQL contains an embedded NUL byte".to_string())?;

    // SAFETY: SPI is entered and finished in the same callback. Tuples needed
    // after `SPI_finish` are copied with `SPI_copytuple` before the tuptable is
    // released.
    unsafe {
        let connect_result = pg_sys::SPI_connect();
        if connect_result != pg_sys::SPI_OK_CONNECT as i32 {
            return Err(format!("SPI_connect failed with code {connect_result}"));
        }

        let execute_result = pg_sys::SPI_execute(c_sql.as_ptr(), true, 0);
        if execute_result != pg_sys::SPI_OK_SELECT as i32 {
            let _ = pg_sys::SPI_finish();
            return Err(format!("SPI_execute failed with code {execute_result}"));
        }

        let processed = pg_sys::SPI_processed;
        let tuptable = pg_sys::SPI_tuptable;
        let mut rows = Vec::new();
        if !tuptable.is_null() {
            let vals = (*tuptable).vals;
            for idx in 0..processed {
                let tuple = *vals.add(idx as usize);
                if !tuple.is_null() {
                    let copied = pg_sys::SPI_copytuple(tuple);
                    if !copied.is_null() {
                        rows.push(copied);
                    }
                }
            }
            pg_sys::SPI_freetuptable(tuptable);
        }

        let finish_result = pg_sys::SPI_finish();
        if finish_result != pg_sys::SPI_OK_FINISH as i32 {
            for tuple in rows {
                if !tuple.is_null() {
                    pg_sys::SPI_freetuple(tuple);
                }
            }
            return Err(format!("SPI_finish failed with code {finish_result}"));
        }

        Ok(RuntimeState { rows, next: 0 })
    }
}

unsafe fn make_private_list(sql: &str) -> *mut pg_sys::List {
    let Ok(c_sql) = CString::new(sql) else {
        return ptr::null_mut();
    };

    // SAFETY: `pstrdup` copies the Rust CString into PostgreSQL planner memory,
    // and `makeString` wraps that palloc'd C string in a serializable Value node.
    let sql_node = unsafe { pg_sys::makeString(pg_sys::pstrdup(c_sql.as_ptr())) };
    if sql_node.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: `lappend` accepts NIL (`NULL`) and returns a PostgreSQL list in
    // the current planner memory context.
    unsafe { pg_sys::lappend(ptr::null_mut(), sql_node.cast()) }
}

unsafe fn private_sql(private: *mut pg_sys::List) -> Option<String> {
    if private.is_null() || pg_sys::list_length(private) != 1 {
        return None;
    }
    let node = pg_sys::list_nth(private, 0).cast::<pg_sys::String>();
    if node.is_null() || (*node).type_ != pg_sys::NodeTag::T_String {
        return None;
    }
    cstr_to_string((*node).sval)
}

unsafe fn limit_rows(root: *mut pg_sys::PlannerInfo) -> Option<i32> {
    let limit = (*root).limit_tuples;
    if !limit.is_finite() || limit <= 0.0 || limit > f64::from(i32::MAX) {
        return None;
    }
    Some(limit.ceil() as i32)
}

unsafe fn is_desc_sort(sortop: pg_sys::Oid) -> bool {
    let opname = pg_sys::get_opname(sortop);
    cstr_to_string(opname).is_some_and(|name| name == ">")
}

unsafe fn is_glot_rank(funcid: pg_sys::Oid) -> bool {
    let fname = pg_sys::get_func_name(funcid);
    let nsp = pg_sys::get_namespace_name(pg_sys::get_func_namespace(funcid));
    cstr_to_string(fname).is_some_and(|name| name == "rank")
        && cstr_to_string(nsp).is_some_and(|name| name == "glot")
}

unsafe fn column_name_for_var(
    relid: pg_sys::Oid,
    rti: pg_sys::Index,
    node: *mut pg_sys::Node,
) -> Option<String> {
    if node.is_null() || (*node).type_ != pg_sys::NodeTag::T_Var {
        return None;
    }
    let var = node.cast::<pg_sys::Var>();
    if (*var).varlevelsup != 0 || (*var).varno != rti as i32 || (*var).varattno <= 0 {
        return None;
    }

    let name = pg_sys::get_attname(relid, (*var).varattno, false);
    cstr_to_string(name)
}

unsafe fn const_i32(node: *mut pg_sys::Node) -> Option<i32> {
    if node.is_null() || (*node).type_ != pg_sys::NodeTag::T_Const {
        return None;
    }
    let konst = node.cast::<pg_sys::Const>();
    if (*konst).constisnull {
        return None;
    }

    if (*konst).consttype == pg_sys::INT4OID {
        Some((*konst).constvalue.value() as i32)
    } else if (*konst).consttype == pg_sys::INT8OID {
        i32::try_from((*konst).constvalue.value() as i64).ok()
    } else {
        None
    }
}

unsafe fn const_to_quoted_literal(node: *mut pg_sys::Node) -> Option<String> {
    if node.is_null() || (*node).type_ != pg_sys::NodeTag::T_Const {
        return None;
    }
    let konst = node.cast::<pg_sys::Const>();
    if (*konst).constisnull {
        return None;
    }

    let mut typoutput = pg_sys::Oid::INVALID;
    let mut typisvarlena = false;
    pg_sys::getTypeOutputInfo((*konst).consttype, &mut typoutput, &mut typisvarlena);
    if typoutput == pg_sys::Oid::INVALID {
        return None;
    }

    let raw = pg_sys::OidOutputFunctionCall(typoutput, (*konst).constvalue);
    quote_literal_cstr_to_string(raw)
}

unsafe fn qualified_relation_name(relid: pg_sys::Oid) -> Option<String> {
    let relname = pg_sys::get_rel_name(relid);
    if relname.is_null() {
        return None;
    }
    let nspname = pg_sys::get_namespace_name(pg_sys::get_rel_namespace(relid));
    if nspname.is_null() {
        return None;
    }
    let qualified = pg_sys::quote_qualified_identifier(nspname, relname);
    cstr_to_string(qualified)
}

unsafe fn quote_literal_cstr_to_string(raw: *mut c_char) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    let quoted = pg_sys::quote_literal_cstr(raw);
    cstr_to_string(quoted)
}

unsafe fn quote_literal_sql(value: &str) -> Option<String> {
    let c_value = CString::new(value).ok()?;
    let quoted = pg_sys::quote_literal_cstr(c_value.as_ptr());
    cstr_to_string(quoted)
}

unsafe fn quote_identifier_sql(value: &str) -> Option<String> {
    let c_value = CString::new(value).ok()?;
    let quoted = pg_sys::quote_identifier(c_value.as_ptr());
    cstr_to_string(quoted)
}

unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}
